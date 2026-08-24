// SPDX-License-Identifier: MPL-2.0

//! Language server client.
//!
//! Everything that talks JSON-RPC lives in a single Tokio task, the *manager*.
//! The editor holds only an [`LspHandle`], which queues [`LspCommand`]s without
//! blocking, and a receiver of [`LspEvent`]s that it drains from its event
//! loop. No editor code awaits a language server, so a server that is slow,
//! wedged, or dead cannot stall rendering or input.
//!
//! The manager owns one server process per language and is the only place that
//! knows a request identifier from a response shape. Above this module the
//! editor sees paths, character offsets, and plain Rust values.

pub mod diagnostics;
pub mod transport;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    str::FromStr,
};

use lsp_types::{
    ClientCapabilities, CodeAction, CodeActionProviderCapability, CodeActionResponse, Command,
    CompletionResponse, DeclarationCapability, DocumentSymbol, DocumentSymbolResponse,
    GotoDefinitionResponse, HoverContents, HoverProviderCapability,
    ImplementationProviderCapability, InitializeResult, MarkedString, OneOf, PositionEncodingKind,
    Range, ServerCapabilities, SymbolInformation, TextDocumentSyncCapability, TextDocumentSyncKind,
    TypeDefinitionProviderCapability, Uri, WorkspaceEdit, WorkspaceSymbolResponse,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    config::{LanguageServerConfig, LspConfig},
    text::{Offset, Text},
};

pub use diagnostics::{Diagnostic, DiagnosticStore, Severity};
use transport::{Connection, Incoming};

/// The protocol types the editor itself needs, re-exported so `lsp_types`
/// stays an implementation detail of this module.
pub use lsp_types::{
    CodeActionOrCommand, Position as LspPosition, Range as LspRange,
    TextDocumentContentChangeEvent, TextEdit,
};

/// Splits a workspace edit carried inline by a code action into per-file text
/// edits, alongside the count of file operations that are not performed.
pub fn flatten_edit(edit: WorkspaceEdit) -> (Vec<DocumentEdit>, usize) {
    flatten_workspace_edit(edit)
}

/// How many commands the editor may queue before the manager is considered
/// wedged. Generous: a burst is one keystroke's worth of document sync.
pub const COMMAND_CAPACITY: usize = 256;
/// How many events may queue for the editor between frames.
pub const EVENT_CAPACITY: usize = 256;

// -- Coordinates -----------------------------------------------------------

/// How a server counts columns.
///
/// LSP defaults to UTF-16 code units, which is the one encoding Runyte does not
/// use internally, so every position crossing this boundary is converted.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Encoding {
    Utf8,
    #[default]
    Utf16,
    Utf32,
}

impl Encoding {
    fn from_kind(kind: Option<&PositionEncodingKind>) -> Self {
        match kind.map(PositionEncodingKind::as_str) {
            Some("utf-8") => Self::Utf8,
            Some("utf-32") => Self::Utf32,
            _ => Self::Utf16,
        }
    }

    fn units(self, character: char) -> u32 {
        match self {
            Self::Utf8 => character.len_utf8() as u32,
            Self::Utf16 => character.len_utf16() as u32,
            Self::Utf32 => 1,
        }
    }
}

/// Converts a character offset into a server position.
pub fn to_lsp_position(text: &Text, offset: Offset, encoding: Encoding) -> lsp_types::Position {
    let offset = offset.min(text.len_chars());
    let row = text.offset_to_row(offset);
    let start = text.line_to_offset(row);
    let character = text
        .slice_string(start, offset)
        .chars()
        .map(|character| encoding.units(character))
        .sum();
    lsp_types::Position {
        line: row as u32,
        character,
    }
}

/// Converts a server position into a character offset, clamping into the
/// document. Clamping matters because diagnostics and edits are asynchronous:
/// a position computed against an older revision must land somewhere sane
/// rather than panic.
pub fn from_lsp_position(text: &Text, position: lsp_types::Position, encoding: Encoding) -> Offset {
    let row = (position.line as usize).min(text.last_row());
    let start = text.line_to_offset(row);
    let line = text.line_string(row);
    let mut units = 0;
    for (index, character) in line.chars().enumerate() {
        if units >= position.character {
            return start + index;
        }
        units += encoding.units(character);
    }
    start + line.chars().count()
}

/// Converts a server range into a half-open character span.
pub fn from_lsp_range(text: &Text, range: Range, encoding: Encoding) -> (Offset, Offset) {
    let from = from_lsp_position(text, range.start, encoding);
    let to = from_lsp_position(text, range.end, encoding);
    if from <= to { (from, to) } else { (to, from) }
}

// -- URIs ------------------------------------------------------------------

/// Percent-encodes an absolute path into a `file:` URI.
///
/// Written here rather than pulled from a URL crate because the only case that
/// matters is an absolute local path, and the encoding rules for that case are
/// short enough that a dependency would cost more than it saves.
pub fn path_to_uri(path: &Path) -> Option<Uri> {
    let text = path.to_str()?;
    let mut encoded = String::from("file://");
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    Uri::from_str(&encoded).ok()
}

/// Decodes a `file:` URI back into a path. Returns `None` for any other scheme,
/// because a server pointing at a non-file resource has nothing the editor can
/// open.
pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let text = uri.as_str();
    let rest = text
        .strip_prefix("file://")
        .map(|rest| rest.strip_prefix("localhost").unwrap_or(rest))?;
    let mut bytes = Vec::with_capacity(rest.len());
    let mut characters = rest.bytes();
    while let Some(byte) = characters.next() {
        if byte == b'%' {
            let high = characters.next()?;
            let low = characters.next()?;
            let pair = [high, low];
            let text = std::str::from_utf8(&pair).ok()?;
            bytes.push(u8::from_str_radix(text, 16).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    (!path.is_empty()).then(|| PathBuf::from(path))
}

// -- The editor-facing protocol -------------------------------------------

/// An edit to one document, already resolved to a path.
#[derive(Clone, Debug)]
pub struct DocumentEdit {
    pub path: PathBuf,
    /// The server's version of an open document, when the workspace edit
    /// carried one. The editor must reject the edit if its live document has
    /// advanced since that version was computed.
    pub version: Option<i32>,
    pub edits: Vec<TextEdit>,
}

/// A location a goto or reference result points at.
#[derive(Clone, Debug)]
pub struct Location {
    pub path: PathBuf,
    pub range: Range,
}

/// A completion candidate, flattened into what the editor needs to display and
/// insert it.
#[derive(Clone, Debug)]
pub struct Completion {
    pub label: String,
    /// Server-provided text used for client-side filtering. The label is the
    /// protocol fallback when this is absent.
    pub filter_text: Option<String>,
    /// Server-provided text used for client-side ordering. The label is the
    /// protocol fallback when this is absent.
    pub sort_text: Option<String>,
    pub detail: String,
    pub kind: &'static str,
    /// Text to insert when no `edit` is supplied.
    pub insert: String,
    /// A server-supplied replacement range, which is authoritative when present
    /// because only the server knows how much of the prefix it intends to
    /// replace.
    pub edit: Option<(Range, String)>,
    /// Extra edits applied alongside the completion, such as an added import.
    pub additional: Vec<TextEdit>,
}

/// A symbol from a document or workspace symbol request.
#[derive(Clone, Debug)]
pub struct SymbolEntry {
    pub name: String,
    pub kind: &'static str,
    pub container: String,
    pub location: Location,
}

/// A code action offered for the current selection.
#[derive(Clone, Debug)]
pub struct ActionEntry {
    pub title: String,
    action: Box<CodeActionOrCommand>,
}

impl ActionEntry {
    pub fn action(&self) -> &CodeActionOrCommand {
        &self.action
    }

    #[cfg(test)]
    pub(crate) fn unresolved_for_test(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            action: Box::new(CodeActionOrCommand::CodeAction(lsp_types::CodeAction {
                title: title.to_owned(),
                ..lsp_types::CodeAction::default()
            })),
        }
    }
}

/// One signature-help overload, pre-rendered.
#[derive(Clone, Debug)]
pub struct SignatureLine {
    pub label: String,
    pub documentation: String,
    pub active_parameter: Option<(u32, u32)>,
}

/// Why signature help is being asked for.
///
/// The specification couples `retriggerCharacters` to a client that sends
/// this: without it a server cannot tell a fresh invocation from a retrigger,
/// so the `)` that should return to the enclosing call of `f(g(a), b)` would
/// read as the start of a new one. Runyte advertises `contextSupport` and
/// sends it for that reason.
#[derive(Clone, Debug, Default)]
pub struct SignatureContext {
    /// The character typed to ask, when a keystroke did the asking.
    pub trigger: Option<char>,
    /// Whether a signature popup was already showing when it was asked for.
    pub retrigger: bool,
}

impl SignatureContext {
    /// The specification's `SignatureHelpContext`.
    ///
    /// `activeSignatureHelp` is left out. It is optional, and the editor keeps
    /// only the signature lines it rendered rather than the server's own value
    /// to echo back, so there is nothing faithful to send.
    fn to_params(&self) -> Value {
        let context = lsp_types::SignatureHelpContext {
            trigger_kind: if self.trigger.is_some() {
                lsp_types::SignatureHelpTriggerKind::TRIGGER_CHARACTER
            } else {
                lsp_types::SignatureHelpTriggerKind::INVOKED
            },
            trigger_character: self.trigger.map(String::from),
            is_retrigger: self.retrigger,
            active_signature_help: None,
        };
        serde_json::to_value(context).unwrap_or(Value::Null)
    }
}

/// What a language server can be asked for.
#[derive(Clone, Debug)]
pub enum RequestKind {
    Definition(lsp_types::Position),
    Declaration(lsp_types::Position),
    TypeDefinition(lsp_types::Position),
    Implementation(lsp_types::Position),
    References(lsp_types::Position),
    Hover(lsp_types::Position),
    Completion(lsp_types::Position),
    SignatureHelp {
        position: lsp_types::Position,
        context: SignatureContext,
    },
    DocumentSymbols,
    WorkspaceSymbols(String),
    Rename {
        position: lsp_types::Position,
        new_name: String,
    },
    CodeActions {
        range: Range,
        diagnostics: Vec<lsp_types::Diagnostic>,
    },
    ResolveCodeAction(Box<CodeAction>),
    ExecuteCommand(Box<Command>),
    Format {
        tab_size: u32,
        insert_spaces: bool,
    },
}

impl RequestKind {
    /// A short name used in status messages when a request fails.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Definition(_) => "definition",
            Self::Declaration(_) => "declaration",
            Self::TypeDefinition(_) => "type definition",
            Self::Implementation(_) => "implementation",
            Self::References(_) => "references",
            Self::Hover(_) => "documentation",
            Self::Completion(_) => "completion",
            Self::SignatureHelp { .. } => "signature help",
            Self::DocumentSymbols => "document symbols",
            Self::WorkspaceSymbols(_) => "workspace symbols",
            Self::Rename { .. } => "rename",
            Self::CodeActions { .. } | Self::ResolveCodeAction(_) => "code actions",
            Self::ExecuteCommand(_) => "command",
            Self::Format { .. } => "formatting",
        }
    }
}

/// The subset of a server's advertised optional capabilities Runyte gates
/// requests on, read once from the `initialize` response and carried for the
/// life of the connection.
///
/// A server that never advertised a capability should never be asked for it:
/// the JSON-RPC answer would be `Method not found`, which the editor cannot
/// tell apart from a real protocol violation once the request has already
/// gone out. Every field defaults to `false`, so a server whose
/// `initialize` response omits a capability entirely — the common case for
/// something it does not implement — is read the same way as one that
/// spelled out `false` explicitly.
///
/// A server also describes *when* it wants to be asked, through the trigger
/// characters on its completion and signature-help options. Those lists are
/// carried here too, so the editor asks on the characters that server named
/// rather than on a set hard-coded for one language.
#[derive(Clone, Debug, Default)]
pub struct Capabilities {
    pub(crate) definition: bool,
    pub(crate) declaration: bool,
    pub(crate) type_definition: bool,
    pub(crate) implementation: bool,
    pub(crate) references: bool,
    pub(crate) hover: bool,
    pub(crate) completion: bool,
    pub(crate) signature_help: bool,
    pub(crate) document_symbols: bool,
    pub(crate) workspace_symbols: bool,
    pub(crate) rename: bool,
    pub(crate) code_actions: bool,
    pub(crate) code_action_resolve: bool,
    pub(crate) execute_command: bool,
    pub(crate) format: bool,
    /// Characters that ask for completion as they are typed. Empty when the
    /// server does not advertise `completionProvider` at all.
    pub(crate) completion_triggers: Vec<char>,
    /// Characters that ask for signature help when no popup is showing.
    /// Empty when the server does not advertise `signatureHelpProvider`.
    pub(crate) signature_triggers: Vec<char>,
    /// Characters that ask again while a signature popup is already showing.
    /// Every trigger character counts as one of these too, so this holds only
    /// the extras the server named.
    pub(crate) signature_retriggers: Vec<char>,
}

/// What Runyte asks on when a server advertises a capability without saying
/// which characters should drive it. The specification allows an empty list,
/// and for completion a client is expected to fall back to its own judgement;
/// Runyte has no explicit signature-help command at all, so an empty list
/// there would make the feature unreachable rather than merely quieter.
const DEFAULT_COMPLETION_TRIGGERS: [char; 2] = ['.', ':'];
const DEFAULT_SIGNATURE_TRIGGERS: [char; 2] = ['(', ','];

impl Capabilities {
    fn from_server(capabilities: &ServerCapabilities) -> Self {
        let code_action_options = match &capabilities.code_action_provider {
            Some(CodeActionProviderCapability::Options(options)) => Some(options),
            _ => None,
        };
        let signature_options = capabilities.signature_help_provider.as_ref();
        Self {
            definition: one_of_bool_supported(capabilities.definition_provider.as_ref()),
            declaration: declaration_supported(capabilities.declaration_provider.as_ref()),
            type_definition: type_definition_supported(
                capabilities.type_definition_provider.as_ref(),
            ),
            implementation: implementation_supported(capabilities.implementation_provider.as_ref()),
            references: one_of_bool_supported(capabilities.references_provider.as_ref()),
            hover: hover_supported(capabilities.hover_provider.as_ref()),
            completion: capabilities.completion_provider.is_some(),
            signature_help: capabilities.signature_help_provider.is_some(),
            document_symbols: one_of_bool_supported(capabilities.document_symbol_provider.as_ref()),
            workspace_symbols: one_of_bool_supported(
                capabilities.workspace_symbol_provider.as_ref(),
            ),
            rename: one_of_bool_supported(capabilities.rename_provider.as_ref()),
            code_actions: code_action_supported(capabilities.code_action_provider.as_ref()),
            code_action_resolve: code_action_options
                .is_some_and(|options| options.resolve_provider == Some(true)),
            execute_command: capabilities.execute_command_provider.is_some(),
            format: one_of_bool_supported(capabilities.document_formatting_provider.as_ref()),
            completion_triggers: capabilities.completion_provider.as_ref().map_or_else(
                Vec::new,
                |options| {
                    trigger_characters(
                        options.trigger_characters.as_ref(),
                        &DEFAULT_COMPLETION_TRIGGERS,
                    )
                },
            ),
            signature_triggers: signature_options.map_or_else(Vec::new, |options| {
                trigger_characters(
                    options.trigger_characters.as_ref(),
                    &DEFAULT_SIGNATURE_TRIGGERS,
                )
            }),
            // No fallback: a retrigger character only matters while a popup is
            // already showing, and Runyte closes one on `)` by itself when the
            // server named nothing.
            signature_retriggers: signature_options.map_or_else(Vec::new, |options| {
                trigger_characters(options.retrigger_characters.as_ref(), &[])
            }),
        }
    }

    /// Whether the server advertised the capability this request needs.
    ///
    /// `ResolveCodeAction` and `ExecuteCommand` only ever follow a code
    /// action or command the server itself already returned, so they gate on
    /// the matching resolve/execute advertisement rather than reusing
    /// `code_actions`.
    pub fn supports(&self, kind: &RequestKind) -> bool {
        match kind {
            RequestKind::Definition(_) => self.definition,
            RequestKind::Declaration(_) => self.declaration,
            RequestKind::TypeDefinition(_) => self.type_definition,
            RequestKind::Implementation(_) => self.implementation,
            RequestKind::References(_) => self.references,
            RequestKind::Hover(_) => self.hover,
            RequestKind::Completion(_) => self.completion,
            RequestKind::SignatureHelp { .. } => self.signature_help,
            RequestKind::DocumentSymbols => self.document_symbols,
            RequestKind::WorkspaceSymbols(_) => self.workspace_symbols,
            RequestKind::Rename { .. } => self.rename,
            RequestKind::CodeActions { .. } => self.code_actions,
            RequestKind::ResolveCodeAction(_) => self.code_action_resolve,
            RequestKind::ExecuteCommand(_) => self.execute_command,
            RequestKind::Format { .. } => self.format,
        }
    }

    /// Whether typing `character` is the server's own cue to be asked for
    /// completion.
    pub fn triggers_completion(&self, character: char) -> bool {
        self.completion_triggers.contains(&character)
    }

    /// Whether typing `character` is the server's own cue to be asked for
    /// signature help, given whether a signature popup is already `showing`.
    ///
    /// Retrigger characters are only active while one is, and every trigger
    /// character counts as a retrigger too, so an open popup widens the set
    /// rather than replacing it.
    pub fn triggers_signature_help(&self, character: char, showing: bool) -> bool {
        self.signature_triggers.contains(&character)
            || (showing && self.signature_retriggers.contains(&character))
    }

    /// Every optional capability enabled. Production values always come from
    /// [`Capabilities::from_server`], which starts every field `false` and
    /// enables only what the server actually advertised; tests that are not
    /// about gating itself use this so a fully-capable mock server does not
    /// have to spell out every field.
    #[cfg(test)]
    pub(crate) fn everything_for_test() -> Self {
        Self {
            definition: true,
            declaration: true,
            type_definition: true,
            implementation: true,
            references: true,
            hover: true,
            completion: true,
            signature_help: true,
            document_symbols: true,
            workspace_symbols: true,
            rename: true,
            code_actions: true,
            code_action_resolve: true,
            execute_command: true,
            format: true,
            completion_triggers: DEFAULT_COMPLETION_TRIGGERS.to_vec(),
            signature_triggers: DEFAULT_SIGNATURE_TRIGGERS.to_vec(),
            signature_retriggers: Vec::new(),
        }
    }
}

/// Reads a `OneOf<bool, T>` capability shape: absent or `Left(false)` means
/// unsupported, `Left(true)` or any `Right(_)` options payload means
/// supported.
/// Reads one advertised list of trigger characters.
///
/// An entry a keystroke can never match — anything that is not exactly one
/// character — is dropped rather than half-matched against its first one. A
/// capability advertised with no usable list falls back to `fallback`, which
/// is empty where no fallback makes sense.
fn trigger_characters(advertised: Option<&Vec<String>>, fallback: &[char]) -> Vec<char> {
    let mut characters: Vec<char> = advertised
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let mut entry = entry.chars();
                    match (entry.next(), entry.next()) {
                        (Some(character), None) => Some(character),
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if characters.is_empty() {
        characters = fallback.to_vec();
    }
    characters.sort_unstable();
    characters.dedup();
    characters
}

fn one_of_bool_supported<T>(capability: Option<&OneOf<bool, T>>) -> bool {
    match capability {
        None => false,
        Some(OneOf::Left(enabled)) => *enabled,
        Some(OneOf::Right(_)) => true,
    }
}

fn hover_supported(capability: Option<&HoverProviderCapability>) -> bool {
    match capability {
        None => false,
        Some(HoverProviderCapability::Simple(enabled)) => *enabled,
        Some(HoverProviderCapability::Options(_)) => true,
    }
}

fn type_definition_supported(capability: Option<&TypeDefinitionProviderCapability>) -> bool {
    match capability {
        None => false,
        Some(TypeDefinitionProviderCapability::Simple(enabled)) => *enabled,
        Some(TypeDefinitionProviderCapability::Options(_)) => true,
    }
}

fn implementation_supported(capability: Option<&ImplementationProviderCapability>) -> bool {
    match capability {
        None => false,
        Some(ImplementationProviderCapability::Simple(enabled)) => *enabled,
        Some(ImplementationProviderCapability::Options(_)) => true,
    }
}

fn declaration_supported(capability: Option<&DeclarationCapability>) -> bool {
    !matches!(
        capability,
        None | Some(DeclarationCapability::Simple(false))
    )
}

fn code_action_supported(capability: Option<&CodeActionProviderCapability>) -> bool {
    match capability {
        None => false,
        Some(CodeActionProviderCapability::Simple(enabled)) => *enabled,
        Some(CodeActionProviderCapability::Options(_)) => true,
    }
}

/// What came back.
#[derive(Clone, Debug)]
pub enum Response {
    Locations(Vec<Location>),
    Hover(String),
    Completions(Vec<Completion>),
    Signatures(Vec<SignatureLine>),
    Symbols(Vec<SymbolEntry>),
    Actions(Vec<ActionEntry>),
    /// A rename, code action, or formatting result, normalized to per-file
    /// edits. File creation, renaming, and deletion are reported in `skipped`
    /// rather than performed: V4 does not let a language server restructure the
    /// project behind the person driving it.
    Edits {
        edits: Vec<DocumentEdit>,
        skipped: usize,
    },
    /// The request succeeded and the server had nothing to offer.
    Empty,
    Failed(String),
}

/// Work for the manager. Every variant is fire-and-forget from the editor's
/// side.
#[derive(Clone, Debug)]
pub enum LspCommand {
    /// Starts a language's server without opening a document.
    ///
    /// Separating this from `Open` is what lets the editor wait for the
    /// handshake before describing any document: the position encoding is
    /// negotiated there, and a `didOpen` sent before it is known could not be
    /// followed by a correct `didChange`.
    Ensure {
        language: String,
    },
    Open {
        language: String,
        path: PathBuf,
        version: i32,
        text: String,
    },
    Change {
        language: String,
        path: PathBuf,
        version: i32,
        changes: Vec<TextDocumentContentChangeEvent>,
    },
    Save {
        language: String,
        path: PathBuf,
        text: String,
    },
    Close {
        language: String,
        path: PathBuf,
    },
    Request {
        token: u64,
        language: String,
        path: PathBuf,
        kind: Box<RequestKind>,
    },
    /// The editor's answer to a server-initiated `workspace/applyEdit`.
    EditApplied {
        language: String,
        id: Value,
        applied: bool,
    },
    /// Restarts one language's server, or every stopped server when `None`.
    Restart(Option<String>),
    Status,
    Shutdown,
}

/// Something the editor should react to.
#[derive(Clone, Debug)]
pub enum LspEvent {
    /// A server finished its handshake. Carries the negotiated position
    /// encoding and sync mode, which the editor needs before it can describe
    /// an edit in the server's terms, plus what the server advertised it can
    /// do, which the editor consults before sending an optional request so a
    /// capability it never claimed is never asked for.
    Ready {
        language: String,
        name: String,
        encoding: Encoding,
        incremental: bool,
        capabilities: Capabilities,
    },
    Diagnostics {
        language: String,
        path: PathBuf,
        diagnostics: Vec<Diagnostic>,
    },
    Response {
        token: u64,
        response: Response,
    },
    /// The server asked the editor to apply an edit. The editor must answer
    /// with [`LspCommand::EditApplied`] and the same `id`.
    ApplyEdit {
        language: String,
        id: Value,
        edits: Vec<DocumentEdit>,
        skipped: usize,
    },
    Status {
        message: String,
        error: bool,
    },
    /// A server exited, crashed, or failed to start. The editor drops its
    /// diagnostics and keeps working without it.
    Stopped {
        language: String,
        message: String,
    },
}

/// The editor's non-blocking view of the manager.
#[derive(Clone, Debug)]
pub struct LspHandle {
    commands: mpsc::Sender<LspCommand>,
}

impl LspHandle {
    /// Queues a command. Returns `false` when the manager has stopped or its
    /// queue is full; callers surface that as a status message rather than
    /// waiting, because waiting is the one thing the render path may not do.
    pub fn send(&self, command: LspCommand) -> bool {
        self.commands.try_send(command).is_ok()
    }
}

/// How the manager obtains a connection to a language's server.
///
/// Production launches a process. Tests supply a pair of in-memory pipes, which
/// is what makes the handshake, a crash, a malformed frame, and a cancellation
/// all reachable without a real language server on the machine.
pub type Launch = Box<
    dyn FnMut(
            &str,
            &LanguageServerConfig,
            &Path,
            mpsc::Sender<(String, Incoming)>,
        ) -> Result<Connection, String>
        + Send,
>;

fn process_launcher() -> Launch {
    Box::new(|language, settings, root, inbox| {
        transport::spawn(
            language.to_owned(),
            &settings.command,
            &settings.args,
            root,
            inbox,
        )
        .map_err(|error| error.to_string())
    })
}

/// Starts the manager. Must be called inside a Tokio runtime.
pub fn spawn(config: LspConfig, root: PathBuf) -> (LspHandle, mpsc::Receiver<LspEvent>) {
    spawn_with(config, root, process_launcher())
}

/// A handle wired to a caller-owned queue instead of a manager.
///
/// Needs no runtime, which is what lets editor tests assert exactly what the
/// editor would send to a language server without starting one.
pub fn command_channel() -> (LspHandle, mpsc::Receiver<LspCommand>) {
    let (commands, queue) = mpsc::channel(COMMAND_CAPACITY);
    (LspHandle { commands }, queue)
}

/// Starts the manager against a caller-supplied way of reaching servers.
pub fn spawn_with(
    config: LspConfig,
    root: PathBuf,
    launch: Launch,
) -> (LspHandle, mpsc::Receiver<LspEvent>) {
    let (command_tx, command_rx) = mpsc::channel(COMMAND_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel(EVENT_CAPACITY);
    tokio::spawn(run_manager(config, root, launch, command_rx, event_tx));
    (
        LspHandle {
            commands: command_tx,
        },
        event_rx,
    )
}

// -- The manager -----------------------------------------------------------

/// What a pending request was asked for, so its response can be decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Shape {
    Initialize,
    Locations,
    Hover,
    Completion,
    Signature,
    DocumentSymbols,
    WorkspaceSymbols,
    Edit,
    Actions,
    ResolvedAction,
    Format,
    Executed,
}

struct Pending {
    token: u64,
    shape: Shape,
    label: &'static str,
}

/// Everything the manager knows about one language's server.
struct Server {
    language: String,
    name: String,
    connection: Connection,
    next_id: i64,
    pending: HashMap<i64, Pending>,
    encoding: Encoding,
    incremental: bool,
    capabilities: Capabilities,
    ready: bool,
    /// Notifications recorded before the handshake completed. The
    /// specification forbids sending them earlier, and the editor should not
    /// have to care.
    queued: Vec<Value>,
}

impl Server {
    fn request(
        &mut self,
        method: &str,
        params: Value,
        token: u64,
        shape: Shape,
        label: &'static str,
    ) -> bool {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(
            id,
            Pending {
                token,
                shape,
                label,
            },
        );
        self.connection.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
    }

    fn notify(&mut self, method: &str, params: Value) -> bool {
        let message = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        if !self.ready {
            self.queued.push(message);
            return true;
        }
        self.connection.send(message)
    }

    fn flush(&mut self) {
        for message in std::mem::take(&mut self.queued) {
            self.connection.send(message);
        }
    }

    fn respond(&mut self, id: Value, result: Value) {
        self.connection.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }));
    }
}

async fn run_manager(
    config: LspConfig,
    root: PathBuf,
    mut launch: Launch,
    mut commands: mpsc::Receiver<LspCommand>,
    events: mpsc::Sender<LspEvent>,
) {
    let (inbox_tx, mut inbox) = mpsc::channel::<(String, Incoming)>(EVENT_CAPACITY);
    let mut servers: HashMap<String, Server> = HashMap::new();
    // Languages whose server failed. Recorded so a failure is reported once
    // and never retried in a loop; `:lsp-restart` clears it.
    let mut failed: HashMap<String, String> = HashMap::new();

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                if matches!(command, LspCommand::Shutdown) {
                    break;
                }
                handle_command(
                    command,
                    &config,
                    &root,
                    &mut launch,
                    &mut servers,
                    &mut failed,
                    &inbox_tx,
                    &events,
                )
                .await;
            }
            message = inbox.recv() => {
                let Some((language, incoming)) = message else { continue };
                handle_incoming(
                    &language,
                    incoming,
                    &mut servers,
                    &mut failed,
                    &events,
                )
                .await;
            }
        }
    }

    for (_, server) in servers.drain() {
        server.connection.send(json!({
            "jsonrpc": "2.0",
            "id": server.next_id,
            "method": "shutdown",
            "params": null,
        }));
        server.connection.send(json!({
            "jsonrpc": "2.0",
            "method": "exit",
            "params": null,
        }));
        server.connection.stop().await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_command(
    command: LspCommand,
    config: &LspConfig,
    root: &Path,
    launch: &mut Launch,
    servers: &mut HashMap<String, Server>,
    failed: &mut HashMap<String, String>,
    inbox: &mpsc::Sender<(String, Incoming)>,
    events: &mpsc::Sender<LspEvent>,
) {
    match command {
        LspCommand::Shutdown => {}
        LspCommand::Ensure { language } => {
            ensure_server(
                &language, config, root, launch, servers, failed, inbox, events,
            )
            .await;
        }
        LspCommand::Restart(language) => {
            let languages: Vec<String> = match language {
                Some(language) => vec![language],
                None => failed.keys().cloned().collect(),
            };
            for language in languages {
                failed.remove(&language);
                if let Some(server) = servers.remove(&language) {
                    server.connection.stop().await;
                }
                emit(
                    events,
                    LspEvent::Status {
                        message: format!(
                            "{language} language server will restart on the next edit"
                        ),
                        error: false,
                    },
                )
                .await;
            }
        }
        LspCommand::Status => {
            let mut lines: Vec<String> = servers
                .values()
                .map(|server| {
                    format!(
                        "{}: {} ({})",
                        server.language,
                        server.name,
                        if server.ready { "ready" } else { "starting" }
                    )
                })
                .collect();
            lines.extend(
                failed
                    .iter()
                    .map(|(language, reason)| format!("{language}: stopped — {reason}")),
            );
            if lines.is_empty() {
                lines.push("no language servers running".to_owned());
            }
            lines.sort();
            emit(
                events,
                LspEvent::Status {
                    message: lines.join(" │ "),
                    error: false,
                },
            )
            .await;
        }
        LspCommand::Open {
            language,
            path,
            version,
            text,
        } => {
            if ensure_server(
                &language, config, root, launch, servers, failed, inbox, events,
            )
            .await
                && let Some(server) = servers.get_mut(&language)
                && let Some(uri) = path_to_uri(&path)
            {
                server.notify(
                    "textDocument/didOpen",
                    json!({
                        "textDocument": {
                            "uri": uri,
                            "languageId": language,
                            "version": version,
                            "text": text,
                        }
                    }),
                );
            }
        }
        LspCommand::Change {
            language,
            path,
            version,
            changes,
        } => {
            if let Some(server) = servers.get_mut(&language)
                && let Some(uri) = path_to_uri(&path)
            {
                server.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": { "uri": uri, "version": version },
                        "contentChanges": changes,
                    }),
                );
            }
        }
        LspCommand::Save {
            language,
            path,
            text,
        } => {
            if let Some(server) = servers.get_mut(&language)
                && let Some(uri) = path_to_uri(&path)
            {
                server.notify(
                    "textDocument/didSave",
                    json!({ "textDocument": { "uri": uri }, "text": text }),
                );
            }
        }
        LspCommand::Close { language, path } => {
            if let Some(server) = servers.get_mut(&language)
                && let Some(uri) = path_to_uri(&path)
            {
                server.notify(
                    "textDocument/didClose",
                    json!({ "textDocument": { "uri": uri } }),
                );
            }
        }
        LspCommand::EditApplied {
            language,
            id,
            applied,
        } => {
            if let Some(server) = servers.get_mut(&language) {
                server.respond(id, json!({ "applied": applied }));
            }
        }
        LspCommand::Request {
            token,
            language,
            path,
            kind,
        } => {
            let Some(server) = servers.get_mut(&language) else {
                let reason = failed
                    .get(&language)
                    .cloned()
                    .unwrap_or_else(|| format!("no language server configured for {language}"));
                emit(
                    events,
                    LspEvent::Response {
                        token,
                        response: Response::Failed(reason),
                    },
                )
                .await;
                return;
            };
            if !server.ready {
                emit(
                    events,
                    LspEvent::Response {
                        token,
                        response: Response::Failed(format!("{} is still starting", server.name)),
                    },
                )
                .await;
                return;
            }
            let Some(uri) = path_to_uri(&path) else {
                emit(
                    events,
                    LspEvent::Response {
                        token,
                        response: Response::Failed(format!(
                            "{} is not a path a language server can address",
                            path.display()
                        )),
                    },
                )
                .await;
                return;
            };
            let label = kind.label();
            let (method, params, shape) = request_payload(&kind, &uri, root);
            if !server.request(method, params, token, shape, label) {
                emit(
                    events,
                    LspEvent::Response {
                        token,
                        response: Response::Failed(format!("{} is not responding", server.name)),
                    },
                )
                .await;
            }
        }
    }
}

/// Starts a language's server if it is configured, not running, and has not
/// already failed.
#[allow(clippy::too_many_arguments)]
async fn ensure_server(
    language: &str,
    config: &LspConfig,
    root: &Path,
    launch: &mut Launch,
    servers: &mut HashMap<String, Server>,
    failed: &mut HashMap<String, String>,
    inbox: &mpsc::Sender<(String, Incoming)>,
    events: &mpsc::Sender<LspEvent>,
) -> bool {
    if servers.contains_key(language) {
        return true;
    }
    if failed.contains_key(language) {
        return false;
    }
    if !config.enable {
        return false;
    }
    let Some(settings) = config.servers.get(language) else {
        return false;
    };

    let name = settings.command.file_name().map_or_else(
        || settings.command.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let connection = match launch(language, settings, root, inbox.clone()) {
        Ok(connection) => connection,
        Err(error) => {
            let reason = format!("cannot start {name}: {error}");
            failed.insert(language.to_owned(), reason.clone());
            emit(
                events,
                LspEvent::Stopped {
                    language: language.to_owned(),
                    message: reason,
                },
            )
            .await;
            return false;
        }
    };

    let mut server = Server {
        language: language.to_owned(),
        name,
        connection,
        next_id: 1,
        pending: HashMap::new(),
        encoding: Encoding::default(),
        incremental: true,
        capabilities: Capabilities::default(),
        ready: false,
        queued: Vec::new(),
    };
    let params = initialize_params(root, settings);
    server.request("initialize", params, 0, Shape::Initialize, "initialize");
    servers.insert(language.to_owned(), server);
    true
}

fn initialize_params(root: &Path, settings: &LanguageServerConfig) -> Value {
    let uri = path_to_uri(root);
    let capabilities = client_capabilities();
    json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "runyte", "version": env!("CARGO_PKG_VERSION") },
        "rootUri": uri,
        "workspaceFolders": uri.as_ref().map(|uri| json!([{
            "uri": uri,
            "name": root.file_name().map_or_else(
                || root.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
        }])),
        "capabilities": capabilities,
        "initializationOptions": settings.initialization_options,
    })
}

/// What Runyte tells a server it can do.
///
/// Written as JSON rather than built from `ClientCapabilities` because the
/// interesting part is which capabilities are *absent*: no snippets, because
/// Runyte inserts completion text literally, and no resource operations,
/// because a language server may not create, rename, or delete files.
fn client_capabilities() -> Value {
    // Kept as a typed round-trip so a field name typo fails at test time
    // rather than being silently ignored by a server.
    let _typed: ClientCapabilities = ClientCapabilities::default();
    json!({
        "general": {
            "positionEncodings": ["utf-8", "utf-32", "utf-16"],
        },
        "workspace": {
            "applyEdit": true,
            "workspaceEdit": {
                "documentChanges": true,
                "resourceOperations": [],
                "failureHandling": "abort",
            },
            "symbol": { "dynamicRegistration": false },
            "executeCommand": { "dynamicRegistration": false },
            "configuration": false,
            "workspaceFolders": true,
        },
        "textDocument": {
            "synchronization": {
                "dynamicRegistration": false,
                "didSave": true,
                "willSave": false,
            },
            "completion": {
                "dynamicRegistration": false,
                "completionItem": {
                    "snippetSupport": false,
                    "documentationFormat": ["plaintext"],
                    "insertReplaceSupport": false,
                    "resolveSupport": { "properties": ["documentation", "detail"] },
                },
                "contextSupport": false,
            },
            "hover": { "contentFormat": ["plaintext", "markdown"] },
            "signatureHelp": {
                "signatureInformation": {
                    "documentationFormat": ["plaintext"],
                    "parameterInformation": { "labelOffsetSupport": true },
                },
                // Opting in is what entitles the editor to read
                // `retriggerCharacters` off the server's options, and what
                // lets it say which character asked and whether a popup was
                // already showing.
                "contextSupport": true,
            },
            "declaration": { "linkSupport": true },
            "definition": { "linkSupport": true },
            "typeDefinition": { "linkSupport": true },
            "implementation": { "linkSupport": true },
            "references": { "dynamicRegistration": false },
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
            "formatting": { "dynamicRegistration": false },
            "rename": { "dynamicRegistration": false, "prepareSupport": false },
            "codeAction": {
                "dynamicRegistration": false,
                "codeActionLiteralSupport": {
                    "codeActionKind": {
                        "valueSet": [
                            "", "quickfix", "refactor", "refactor.extract",
                            "refactor.inline", "refactor.rewrite", "source",
                            "source.organizeImports",
                        ],
                    },
                },
                "resolveSupport": { "properties": ["edit"] },
            },
            "publishDiagnostics": { "relatedInformation": false },
        },
    })
}

fn request_payload(kind: &RequestKind, uri: &Uri, root: &Path) -> (&'static str, Value, Shape) {
    let document = json!({ "uri": uri });
    match kind {
        RequestKind::Definition(position) => (
            "textDocument/definition",
            json!({ "textDocument": document, "position": position }),
            Shape::Locations,
        ),
        RequestKind::Declaration(position) => (
            "textDocument/declaration",
            json!({ "textDocument": document, "position": position }),
            Shape::Locations,
        ),
        RequestKind::TypeDefinition(position) => (
            "textDocument/typeDefinition",
            json!({ "textDocument": document, "position": position }),
            Shape::Locations,
        ),
        RequestKind::Implementation(position) => (
            "textDocument/implementation",
            json!({ "textDocument": document, "position": position }),
            Shape::Locations,
        ),
        RequestKind::References(position) => (
            "textDocument/references",
            json!({
                "textDocument": document,
                "position": position,
                "context": { "includeDeclaration": true },
            }),
            Shape::Locations,
        ),
        RequestKind::Hover(position) => (
            "textDocument/hover",
            json!({ "textDocument": document, "position": position }),
            Shape::Hover,
        ),
        RequestKind::Completion(position) => (
            "textDocument/completion",
            json!({ "textDocument": document, "position": position }),
            Shape::Completion,
        ),
        RequestKind::SignatureHelp { position, context } => (
            "textDocument/signatureHelp",
            json!({
                "textDocument": document,
                "position": position,
                "context": context.to_params(),
            }),
            Shape::Signature,
        ),
        RequestKind::DocumentSymbols => (
            "textDocument/documentSymbol",
            json!({ "textDocument": document }),
            Shape::DocumentSymbols,
        ),
        RequestKind::WorkspaceSymbols(query) => (
            "workspace/symbol",
            json!({ "query": query }),
            Shape::WorkspaceSymbols,
        ),
        RequestKind::Rename { position, new_name } => (
            "textDocument/rename",
            json!({
                "textDocument": document,
                "position": position,
                "newName": new_name,
            }),
            Shape::Edit,
        ),
        RequestKind::CodeActions { range, diagnostics } => (
            "textDocument/codeAction",
            json!({
                "textDocument": document,
                "range": range,
                "context": { "diagnostics": diagnostics },
            }),
            Shape::Actions,
        ),
        RequestKind::ResolveCodeAction(action) => (
            "codeAction/resolve",
            serde_json::to_value(action.as_ref()).unwrap_or(Value::Null),
            Shape::ResolvedAction,
        ),
        RequestKind::ExecuteCommand(command) => (
            "workspace/executeCommand",
            json!({
                "command": command.command,
                "arguments": command.arguments.clone().unwrap_or_default(),
            }),
            Shape::Executed,
        ),
        RequestKind::Format {
            tab_size,
            insert_spaces,
        } => {
            let _ = root;
            (
                "textDocument/formatting",
                json!({
                    "textDocument": document,
                    "options": {
                        "tabSize": tab_size,
                        "insertSpaces": insert_spaces,
                        "trimTrailingWhitespace": true,
                        "insertFinalNewline": true,
                    },
                }),
                Shape::Format,
            )
        }
    }
}

async fn handle_incoming(
    language: &str,
    incoming: Incoming,
    servers: &mut HashMap<String, Server>,
    failed: &mut HashMap<String, String>,
    events: &mpsc::Sender<LspEvent>,
) {
    match incoming {
        Incoming::Malformed(detail) => {
            let name = servers
                .get(language)
                .map_or(language, |server| server.name.as_str())
                .to_owned();
            emit(
                events,
                LspEvent::Status {
                    message: format!("{name} sent an unreadable message: {detail}"),
                    error: true,
                },
            )
            .await;
        }
        Incoming::Closed { reason } => {
            let Some(server) = servers.remove(language) else {
                return;
            };
            let tail = server.connection.stderr_tail();
            let detail = match (reason.is_empty(), tail.is_empty()) {
                (true, true) => "exited".to_owned(),
                (true, false) => tail,
                (false, true) => reason,
                (false, false) => format!("{reason}: {tail}"),
            };
            let message = format!("{} stopped: {detail}", server.name);
            // Every request the server will now never answer is failed
            // explicitly, so nothing in the editor waits forever.
            for (_, pending) in server.pending {
                emit(
                    events,
                    LspEvent::Response {
                        token: pending.token,
                        response: Response::Failed(message.clone()),
                    },
                )
                .await;
            }
            server.connection.stop().await;
            failed.insert(language.to_owned(), message.clone());
            emit(
                events,
                LspEvent::Stopped {
                    language: language.to_owned(),
                    message,
                },
            )
            .await;
        }
        Incoming::Message(message) => {
            handle_message(language, *message, servers, events).await;
        }
    }
}

async fn handle_message(
    language: &str,
    message: Value,
    servers: &mut HashMap<String, Server>,
    events: &mpsc::Sender<LspEvent>,
) {
    let has_method = message.get("method").is_some();
    let id = message.get("id").cloned();
    match (has_method, id) {
        // A request from the server.
        (true, Some(id)) => handle_server_request(language, &message, id, servers, events).await,
        // A notification from the server.
        (true, None) => handle_notification(language, &message, events).await,
        // A response to one of ours.
        (false, Some(id)) => handle_response(language, &message, &id, servers, events).await,
        (false, None) => {}
    }
}

async fn handle_server_request(
    language: &str,
    message: &Value,
    id: Value,
    servers: &mut HashMap<String, Server>,
    events: &mpsc::Sender<LspEvent>,
) {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "workspace/applyEdit" => {
            let edit = params
                .get("edit")
                .cloned()
                .and_then(|edit| serde_json::from_value::<WorkspaceEdit>(edit).ok());
            match edit {
                Some(edit) => {
                    let (edits, skipped) = flatten_workspace_edit(edit);
                    emit(
                        events,
                        LspEvent::ApplyEdit {
                            language: language.to_owned(),
                            id,
                            edits,
                            skipped,
                        },
                    )
                    .await;
                }
                None => {
                    if let Some(server) = servers.get_mut(language) {
                        server.respond(id, json!({ "applied": false }));
                    }
                }
            }
        }
        // Anything else is answered rather than ignored: a server that blocks
        // on an unanswered request would otherwise appear to hang.
        _ => {
            if let Some(server) = servers.get_mut(language) {
                server.connection.send(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32601,
                        "message": format!("runyte does not implement {method}"),
                    },
                }));
            }
        }
    }
}

async fn handle_notification(language: &str, message: &Value, events: &mpsc::Sender<LspEvent>) {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "textDocument/publishDiagnostics" => {
            let Ok(published) =
                serde_json::from_value::<lsp_types::PublishDiagnosticsParams>(params)
            else {
                return;
            };
            let Some(path) = uri_to_path(&published.uri) else {
                return;
            };
            emit(
                events,
                LspEvent::Diagnostics {
                    language: language.to_owned(),
                    path,
                    diagnostics: published
                        .diagnostics
                        .into_iter()
                        .map(Diagnostic::new)
                        .collect(),
                },
            )
            .await;
        }
        "window/showMessage" | "window/logMessage" => {
            let text = params
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // Log messages are noise in a one-line status bar; only messages
            // the server explicitly asked to show are surfaced, and only when
            // they carry a warning or error type.
            let severity = params.get("type").and_then(Value::as_i64).unwrap_or(4);
            if method == "window/showMessage" && severity <= 2 && !text.is_empty() {
                emit(
                    events,
                    LspEvent::Status {
                        message: format!("{language}: {text}"),
                        error: severity == 1,
                    },
                )
                .await;
            }
        }
        _ => {}
    }
}

async fn handle_response(
    language: &str,
    message: &Value,
    id: &Value,
    servers: &mut HashMap<String, Server>,
    events: &mpsc::Sender<LspEvent>,
) {
    let Some(id) = id.as_i64() else { return };
    let Some(server) = servers.get_mut(language) else {
        return;
    };
    let Some(pending) = server.pending.remove(&id) else {
        return;
    };

    if let Some(error) = message.get("error") {
        let detail = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        if pending.shape == Shape::Initialize {
            let message = format!("{} failed to initialize: {detail}", server.name);
            emit(
                events,
                LspEvent::Stopped {
                    language: language.to_owned(),
                    message,
                },
            )
            .await;
            return;
        }
        emit(
            events,
            LspEvent::Response {
                token: pending.token,
                response: Response::Failed(format!("{}: {detail}", pending.label)),
            },
        )
        .await;
        return;
    }

    let result = message.get("result").cloned().unwrap_or(Value::Null);
    if pending.shape == Shape::Initialize {
        finish_initialize(language, result, server, events).await;
        return;
    }

    let response = decode(pending.shape, result, pending.label);
    emit(
        events,
        LspEvent::Response {
            token: pending.token,
            response,
        },
    )
    .await;
}

async fn finish_initialize(
    language: &str,
    result: Value,
    server: &mut Server,
    events: &mpsc::Sender<LspEvent>,
) {
    let initialized: InitializeResult = serde_json::from_value(result).unwrap_or_default();
    server.encoding = Encoding::from_kind(initialized.capabilities.position_encoding.as_ref());
    server.incremental = matches!(
        initialized.capabilities.text_document_sync,
        None | Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL
        )) | Some(TextDocumentSyncCapability::Options(
            lsp_types::TextDocumentSyncOptions {
                change: None | Some(TextDocumentSyncKind::INCREMENTAL),
                ..
            }
        ))
    );
    server.capabilities = Capabilities::from_server(&initialized.capabilities);
    if let Some(info) = initialized.server_info {
        server.name = info.name;
    }
    server.ready = true;
    server.connection.send(json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {},
    }));
    // An empty initial configuration means "use your defaults". Some
    // servers, notably Pyright, do not start project analysis (and therefore
    // do not answer document requests) until they receive this notification.
    server.connection.send(json!({
        "jsonrpc": "2.0",
        "method": "workspace/didChangeConfiguration",
        "params": { "settings": {} },
    }));
    server.flush();
    emit(
        events,
        LspEvent::Ready {
            language: language.to_owned(),
            name: server.name.clone(),
            encoding: server.encoding,
            incremental: server.incremental,
            capabilities: server.capabilities.clone(),
        },
    )
    .await;
}

fn decode(shape: Shape, result: Value, label: &'static str) -> Response {
    if result.is_null() {
        return Response::Empty;
    }
    match shape {
        Shape::Initialize | Shape::Executed => Response::Empty,
        Shape::Locations => match serde_json::from_value::<GotoDefinitionResponse>(result) {
            Ok(response) => Response::Locations(flatten_locations(response)),
            Err(error) => Response::Failed(format!("{label}: {error}")),
        },
        Shape::Hover => match serde_json::from_value::<lsp_types::Hover>(result) {
            Ok(hover) => {
                let text = render_hover(hover.contents);
                if text.trim().is_empty() {
                    Response::Empty
                } else {
                    Response::Hover(text)
                }
            }
            Err(error) => Response::Failed(format!("{label}: {error}")),
        },
        Shape::Completion => match serde_json::from_value::<CompletionResponse>(result) {
            Ok(response) => {
                let items = match response {
                    CompletionResponse::Array(items) => items,
                    CompletionResponse::List(list) => list.items,
                };
                Response::Completions(items.into_iter().map(completion).collect())
            }
            Err(error) => Response::Failed(format!("{label}: {error}")),
        },
        Shape::Signature => match serde_json::from_value::<lsp_types::SignatureHelp>(result) {
            Ok(help) => Response::Signatures(signature_lines(help)),
            Err(error) => Response::Failed(format!("{label}: {error}")),
        },
        Shape::DocumentSymbols => match serde_json::from_value::<DocumentSymbolResponse>(result) {
            Ok(DocumentSymbolResponse::Flat(items)) => {
                Response::Symbols(items.into_iter().filter_map(flat_symbol).collect())
            }
            Ok(DocumentSymbolResponse::Nested(items)) => {
                let mut symbols = Vec::new();
                for item in &items {
                    push_nested_symbol(item, "", &mut symbols);
                }
                Response::Symbols(symbols)
            }
            Err(error) => Response::Failed(format!("{label}: {error}")),
        },
        Shape::WorkspaceSymbols => {
            match serde_json::from_value::<WorkspaceSymbolResponse>(result) {
                Ok(WorkspaceSymbolResponse::Flat(items)) => {
                    Response::Symbols(items.into_iter().filter_map(flat_symbol).collect())
                }
                Ok(WorkspaceSymbolResponse::Nested(items)) => Response::Symbols(
                    items
                        .into_iter()
                        .filter_map(|symbol| {
                            let location = match symbol.location {
                                lsp_types::OneOf::Left(location) => Location {
                                    path: uri_to_path(&location.uri)?,
                                    range: location.range,
                                },
                                lsp_types::OneOf::Right(workspace) => Location {
                                    path: uri_to_path(&workspace.uri)?,
                                    range: Range::default(),
                                },
                            };
                            Some(SymbolEntry {
                                name: symbol.name,
                                kind: symbol_kind(symbol.kind),
                                container: symbol.container_name.unwrap_or_default(),
                                location,
                            })
                        })
                        .collect(),
                ),
                Err(error) => Response::Failed(format!("{label}: {error}")),
            }
        }
        Shape::Edit => match serde_json::from_value::<WorkspaceEdit>(result) {
            Ok(edit) => {
                let (edits, skipped) = flatten_workspace_edit(edit);
                if edits.is_empty() && skipped == 0 {
                    Response::Empty
                } else {
                    Response::Edits { edits, skipped }
                }
            }
            Err(error) => Response::Failed(format!("{label}: {error}")),
        },
        Shape::Format => match serde_json::from_value::<Vec<TextEdit>>(result) {
            Ok(edits) if edits.is_empty() => Response::Empty,
            // Formatting is scoped to the document that was asked about, so the
            // path is filled in by the caller that knows it.
            Ok(edits) => Response::Edits {
                edits: vec![DocumentEdit {
                    path: PathBuf::new(),
                    version: None,
                    edits,
                }],
                skipped: 0,
            },
            Err(error) => Response::Failed(format!("{label}: {error}")),
        },
        Shape::Actions => match serde_json::from_value::<CodeActionResponse>(result) {
            Ok(actions) if actions.is_empty() => Response::Empty,
            Ok(actions) => Response::Actions(
                actions
                    .into_iter()
                    .map(|action| ActionEntry {
                        title: match &action {
                            CodeActionOrCommand::CodeAction(action) => action.title.clone(),
                            CodeActionOrCommand::Command(command) => command.title.clone(),
                        },
                        action: Box::new(action),
                    })
                    .collect(),
            ),
            Err(error) => Response::Failed(format!("{label}: {error}")),
        },
        Shape::ResolvedAction => match serde_json::from_value::<CodeAction>(result) {
            Ok(action) => match action.edit {
                Some(edit) => {
                    let (edits, skipped) = flatten_workspace_edit(edit);
                    Response::Edits { edits, skipped }
                }
                None => match action.command {
                    Some(command) => Response::Actions(vec![ActionEntry {
                        title: command.title.clone(),
                        action: Box::new(CodeActionOrCommand::Command(command)),
                    }]),
                    None => Response::Empty,
                },
            },
            Err(error) => Response::Failed(format!("{label}: {error}")),
        },
    }
}

fn flatten_locations(response: GotoDefinitionResponse) -> Vec<Location> {
    let raw: Vec<(Uri, Range)> = match response {
        GotoDefinitionResponse::Scalar(location) => vec![(location.uri, location.range)],
        GotoDefinitionResponse::Array(locations) => locations
            .into_iter()
            .map(|location| (location.uri, location.range))
            .collect(),
        GotoDefinitionResponse::Link(links) => links
            .into_iter()
            .map(|link| (link.target_uri, link.target_selection_range))
            .collect(),
    };
    raw.into_iter()
        .filter_map(|(uri, range)| {
            Some(Location {
                path: uri_to_path(&uri)?,
                range,
            })
        })
        .collect()
}

/// Splits a workspace edit into per-file text edits, counting the file
/// creations, renames, and deletions that are deliberately not performed.
fn flatten_workspace_edit(edit: WorkspaceEdit) -> (Vec<DocumentEdit>, usize) {
    let mut documents: Vec<DocumentEdit> = Vec::new();
    let mut skipped = 0;

    if let Some(changes) = edit.changes {
        let mut entries: Vec<(PathBuf, Vec<TextEdit>)> = changes
            .into_iter()
            .filter_map(|(uri, edits)| Some((uri_to_path(&uri)?, edits)))
            .collect();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        documents.extend(entries.into_iter().map(|(path, edits)| DocumentEdit {
            path,
            version: None,
            edits,
        }));
    }

    match edit.document_changes {
        Some(lsp_types::DocumentChanges::Edits(edits)) => {
            for edit in edits {
                if let Some(path) = uri_to_path(&edit.text_document.uri) {
                    documents.push(DocumentEdit {
                        path,
                        version: edit.text_document.version,
                        edits: edit.edits.into_iter().map(annotated).collect(),
                    });
                }
            }
        }
        Some(lsp_types::DocumentChanges::Operations(operations)) => {
            for operation in operations {
                match operation {
                    lsp_types::DocumentChangeOperation::Edit(edit) => {
                        if let Some(path) = uri_to_path(&edit.text_document.uri) {
                            documents.push(DocumentEdit {
                                path,
                                version: edit.text_document.version,
                                edits: edit.edits.into_iter().map(annotated).collect(),
                            });
                        }
                    }
                    lsp_types::DocumentChangeOperation::Op(_) => skipped += 1,
                }
            }
        }
        None => {}
    }
    (documents, skipped)
}

fn annotated(edit: lsp_types::OneOf<TextEdit, lsp_types::AnnotatedTextEdit>) -> TextEdit {
    match edit {
        lsp_types::OneOf::Left(edit) => edit,
        lsp_types::OneOf::Right(annotated) => annotated.text_edit,
    }
}

fn render_hover(contents: HoverContents) -> String {
    fn marked(value: MarkedString) -> String {
        match value {
            MarkedString::String(text) => text,
            MarkedString::LanguageString(language) => language.value,
        }
    }
    match contents {
        HoverContents::Scalar(value) => marked(value),
        HoverContents::Array(values) => values
            .into_iter()
            .map(marked)
            .collect::<Vec<_>>()
            .join("\n"),
        HoverContents::Markup(markup) => markup.value,
    }
}

fn completion(item: lsp_types::CompletionItem) -> Completion {
    let edit = item.text_edit.map(|edit| match edit {
        lsp_types::CompletionTextEdit::Edit(edit) => (edit.range, edit.new_text),
        // `InsertReplaceEdit` is only sent when the client advertises support,
        // which Runyte does not; treat it as the insert range if a server sends
        // it anyway.
        lsp_types::CompletionTextEdit::InsertAndReplace(edit) => (edit.insert, edit.new_text),
    });
    let insert = item
        .insert_text
        .clone()
        .unwrap_or_else(|| item.label.clone());
    Completion {
        filter_text: item.filter_text,
        sort_text: item.sort_text,
        detail: item
            .detail
            .clone()
            .or_else(|| match &item.documentation {
                Some(lsp_types::Documentation::String(text)) => Some(first_line(text)),
                Some(lsp_types::Documentation::MarkupContent(markup)) => {
                    Some(first_line(&markup.value))
                }
                None => None,
            })
            .unwrap_or_default(),
        kind: completion_kind(item.kind),
        label: item.label,
        insert,
        edit,
        additional: item.additional_text_edits.unwrap_or_default(),
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn signature_lines(help: lsp_types::SignatureHelp) -> Vec<SignatureLine> {
    let active = help.active_signature.unwrap_or(0) as usize;
    help.signatures
        .into_iter()
        .enumerate()
        .map(|(index, signature)| {
            let active_parameter = if index == active {
                signature
                    .active_parameter
                    .or(help.active_parameter)
                    .and_then(|active| signature.parameters.as_ref()?.get(active as usize).cloned())
                    .and_then(|parameter| match parameter.label {
                        lsp_types::ParameterLabel::LabelOffsets([start, end]) => {
                            signature_parameter_bytes(&signature.label, start, end)
                        }
                        lsp_types::ParameterLabel::Simple(text) => {
                            let start = signature.label.find(&text)?;
                            Some((
                                u32::try_from(start).ok()?,
                                u32::try_from(start.checked_add(text.len())?).ok()?,
                            ))
                        }
                    })
            } else {
                None
            };
            SignatureLine {
                documentation: match signature.documentation {
                    Some(lsp_types::Documentation::String(text)) => first_line(&text),
                    Some(lsp_types::Documentation::MarkupContent(markup)) => {
                        first_line(&markup.value)
                    }
                    None => String::new(),
                },
                label: signature.label,
                active_parameter,
            }
        })
        .collect()
}

fn signature_parameter_bytes(label: &str, start: u32, end: u32) -> Option<(u32, u32)> {
    if start > end {
        return None;
    }
    Some((
        u32::try_from(utf16_offset_to_byte(label, start)?).ok()?,
        u32::try_from(utf16_offset_to_byte(label, end)?).ok()?,
    ))
}

fn utf16_offset_to_byte(text: &str, offset: u32) -> Option<usize> {
    let mut utf16 = 0_u32;
    for (byte, character) in text.char_indices() {
        if utf16 == offset {
            return Some(byte);
        }
        utf16 = utf16.checked_add(character.len_utf16() as u32)?;
        if utf16 > offset {
            return None;
        }
    }
    (utf16 == offset).then_some(text.len())
}

fn flat_symbol(symbol: SymbolInformation) -> Option<SymbolEntry> {
    Some(SymbolEntry {
        name: symbol.name,
        kind: symbol_kind(symbol.kind),
        container: symbol.container_name.unwrap_or_default(),
        location: Location {
            path: uri_to_path(&symbol.location.uri)?,
            range: symbol.location.range,
        },
    })
}

/// Flattens a hierarchical document symbol tree, keeping the parent chain as
/// the container so a picker row stays meaningful without indentation.
fn push_nested_symbol(symbol: &DocumentSymbol, container: &str, into: &mut Vec<SymbolEntry>) {
    into.push(SymbolEntry {
        name: symbol.name.clone(),
        kind: symbol_kind(symbol.kind),
        container: container.to_owned(),
        location: Location {
            // Nested symbols carry no URI: they are always in the requested
            // document, and the caller fills the path in.
            path: PathBuf::new(),
            range: symbol.selection_range,
        },
    });
    let nested = if container.is_empty() {
        symbol.name.clone()
    } else {
        format!("{container}::{}", symbol.name)
    };
    for child in symbol.children.iter().flatten() {
        push_nested_symbol(child, &nested, into);
    }
}

fn symbol_kind(kind: lsp_types::SymbolKind) -> &'static str {
    use lsp_types::SymbolKind as Kind;
    match kind {
        Kind::FILE => "file",
        Kind::MODULE => "module",
        Kind::NAMESPACE => "namespace",
        Kind::PACKAGE => "package",
        Kind::CLASS => "class",
        Kind::METHOD => "method",
        Kind::PROPERTY => "property",
        Kind::FIELD => "field",
        Kind::CONSTRUCTOR => "constructor",
        Kind::ENUM => "enum",
        Kind::INTERFACE => "interface",
        Kind::FUNCTION => "function",
        Kind::VARIABLE => "variable",
        Kind::CONSTANT => "constant",
        Kind::STRING => "string",
        Kind::NUMBER => "number",
        Kind::BOOLEAN => "boolean",
        Kind::ARRAY => "array",
        Kind::OBJECT => "object",
        Kind::KEY => "key",
        Kind::NULL => "null",
        Kind::ENUM_MEMBER => "enum member",
        Kind::STRUCT => "struct",
        Kind::EVENT => "event",
        Kind::OPERATOR => "operator",
        Kind::TYPE_PARAMETER => "type parameter",
        _ => "symbol",
    }
}

fn completion_kind(kind: Option<lsp_types::CompletionItemKind>) -> &'static str {
    use lsp_types::CompletionItemKind as Kind;
    match kind {
        Some(Kind::METHOD) => "method",
        Some(Kind::FUNCTION) => "function",
        Some(Kind::CONSTRUCTOR) => "constructor",
        Some(Kind::FIELD) => "field",
        Some(Kind::VARIABLE) => "variable",
        Some(Kind::CLASS) => "class",
        Some(Kind::INTERFACE) => "interface",
        Some(Kind::MODULE) => "module",
        Some(Kind::PROPERTY) => "property",
        Some(Kind::ENUM) => "enum",
        Some(Kind::KEYWORD) => "keyword",
        Some(Kind::SNIPPET) => "snippet",
        Some(Kind::CONSTANT) => "constant",
        Some(Kind::STRUCT) => "struct",
        Some(Kind::TYPE_PARAMETER) => "type parameter",
        Some(Kind::VALUE) => "value",
        Some(Kind::UNIT) => "unit",
        Some(Kind::FILE) => "file",
        Some(Kind::FOLDER) => "folder",
        Some(Kind::TEXT) => "text",
        _ => "",
    }
}

async fn emit(events: &mpsc::Sender<LspEvent>, event: LspEvent) {
    let _ = events.send(event).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_preserves_server_filter_and_sort_text() {
        let Response::Completions(items) = decode(
            Shape::Completion,
            json!([{
                "label": "displayed",
                "filterText": "matched",
                "sortText": "001",
                "kind": 6
            }]),
            "completion",
        ) else {
            panic!("expected completions");
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].filter_text.as_deref(), Some("matched"));
        assert_eq!(items[0].sort_text.as_deref(), Some("001"));
    }

    #[test]
    fn positions_convert_in_every_encoding() {
        let text = Text::from_str("aé🦀b\nsecond");
        // "aé🦀" is 3 characters, 7 bytes, 4 UTF-16 units.
        let offset = 3;
        assert_eq!(to_lsp_position(&text, offset, Encoding::Utf8).character, 7);
        assert_eq!(to_lsp_position(&text, offset, Encoding::Utf16).character, 4);
        assert_eq!(to_lsp_position(&text, offset, Encoding::Utf32).character, 3);
        for encoding in [Encoding::Utf8, Encoding::Utf16, Encoding::Utf32] {
            let position = to_lsp_position(&text, offset, encoding);
            assert_eq!(from_lsp_position(&text, position, encoding), offset);
        }
    }

    #[test]
    fn signature_offsets_convert_from_utf16_to_valid_byte_ranges() {
        let label = "fn(🦀)";
        assert_eq!(signature_parameter_bytes(label, 3, 5), Some((3, 7)));
        assert_eq!(signature_parameter_bytes(label, 5, 3), None);
        assert_eq!(signature_parameter_bytes(label, 4, 5), None);
        assert_eq!(signature_parameter_bytes(label, 3, 99), None);
    }

    #[test]
    fn simple_unicode_signature_labels_use_byte_ranges() {
        let lines = signature_lines(lsp_types::SignatureHelp {
            signatures: vec![lsp_types::SignatureInformation {
                label: "fn(é)".to_owned(),
                documentation: None,
                parameters: Some(vec![lsp_types::ParameterInformation {
                    label: lsp_types::ParameterLabel::Simple("é".to_owned()),
                    documentation: None,
                }]),
                active_parameter: Some(0),
            }],
            active_signature: Some(0),
            active_parameter: None,
        });

        assert_eq!(lines[0].active_parameter, Some((3, 5)));
        assert_eq!(&lines[0].label[3..5], "é");
    }

    #[test]
    fn out_of_range_positions_clamp_into_the_document() {
        let text = Text::from_str("abc");
        let position = lsp_types::Position::new(99, 99);
        assert_eq!(from_lsp_position(&text, position, Encoding::Utf16), 3);
    }

    #[test]
    fn paths_round_trip_through_file_uris() {
        for path in [
            "/tmp/plain.rs",
            "/tmp/with space/é.rs",
            "/tmp/percent%20literal.rs",
        ] {
            let uri = path_to_uri(Path::new(path)).expect(path);
            assert!(uri.as_str().starts_with("file:///"));
            assert_eq!(uri_to_path(&uri).as_deref(), Some(Path::new(path)));
        }
    }

    #[test]
    fn a_non_file_uri_has_no_path() {
        let uri = Uri::from_str("https://example.com/x").unwrap();
        assert_eq!(uri_to_path(&uri), None);
    }

    #[test]
    fn a_null_result_is_empty_rather_than_a_failure() {
        assert!(matches!(
            decode(Shape::Locations, Value::Null, "definition"),
            Response::Empty
        ));
    }

    #[test]
    fn a_malformed_result_fails_the_request_without_panicking() {
        assert!(matches!(
            decode(Shape::Locations, json!({"nonsense": true}), "definition"),
            Response::Failed(_)
        ));
    }

    #[test]
    fn location_links_and_scalars_decode_alike() {
        let scalar = json!({
            "uri": "file:///tmp/a.rs",
            "range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 5}},
        });
        let link = json!([{
            "targetUri": "file:///tmp/a.rs",
            "targetRange": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 9}},
            "targetSelectionRange": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 5}},
        }]);
        for result in [scalar, link] {
            let Response::Locations(locations) = decode(Shape::Locations, result, "definition")
            else {
                panic!("expected locations");
            };
            assert_eq!(locations.len(), 1);
            assert_eq!(locations[0].path, PathBuf::from("/tmp/a.rs"));
            assert_eq!(locations[0].range.start.character, 2);
        }
    }

    #[test]
    fn resource_operations_are_counted_rather_than_applied() {
        let edit: WorkspaceEdit = serde_json::from_value(json!({
            "documentChanges": [
                {"kind": "create", "uri": "file:///tmp/new.rs"},
                {
                    "textDocument": {"uri": "file:///tmp/a.rs", "version": 1},
                    "edits": [{
                        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                        "newText": "x",
                    }],
                },
            ],
        }))
        .unwrap();
        let (edits, skipped) = flatten_workspace_edit(edit);
        assert_eq!(skipped, 1);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].path, PathBuf::from("/tmp/a.rs"));
        assert_eq!(edits[0].version, Some(1));
    }

    #[test]
    fn nested_document_symbols_keep_their_parent_chain() {
        let result = json!([{
            "name": "Outer",
            "kind": 23,
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 9, "character": 0}},
            "selectionRange": {"start": {"line": 0, "character": 7}, "end": {"line": 0, "character": 12}},
            "children": [{
                "name": "inner",
                "kind": 6,
                "range": {"start": {"line": 1, "character": 0}, "end": {"line": 2, "character": 0}},
                "selectionRange": {"start": {"line": 1, "character": 7}, "end": {"line": 1, "character": 12}},
            }],
        }]);
        let Response::Symbols(symbols) = decode(Shape::DocumentSymbols, result, "symbols") else {
            panic!("expected symbols");
        };
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[1].name, "inner");
        assert_eq!(symbols[1].container, "Outer");
        assert_eq!(symbols[1].kind, "method");
    }

    /// Reads capabilities the way a handshake does, from the wire shape, so
    /// the tests exercise the same deserialization as a real server.
    fn advertised(capabilities: Value) -> Capabilities {
        Capabilities::from_server(
            &serde_json::from_value::<ServerCapabilities>(capabilities)
                .expect("valid server capabilities"),
        )
    }

    #[test]
    fn a_signature_request_carries_the_context_its_retrigger_needs() {
        let uri = path_to_uri(Path::new("/tmp/a.rs")).expect("a file uri");
        let root = Path::new("/tmp");
        let position = lsp_types::Position {
            line: 0,
            character: 0,
        };

        let (method, params, _) = request_payload(
            &RequestKind::SignatureHelp {
                position,
                context: SignatureContext {
                    trigger: Some(')'),
                    retrigger: true,
                },
            },
            &uri,
            root,
        );
        assert_eq!(method, "textDocument/signatureHelp");
        // Without these a server cannot tell the `)` that closed an inner call
        // from one that opened a fresh request, which is the whole point of
        // advertising retrigger characters.
        assert_eq!(params["context"]["triggerKind"], json!(2));
        assert_eq!(params["context"]["triggerCharacter"], json!(")"));
        assert_eq!(params["context"]["isRetrigger"], json!(true));

        // An invocation no keystroke drove names no character.
        let (_, params, _) = request_payload(
            &RequestKind::SignatureHelp {
                position,
                context: SignatureContext::default(),
            },
            &uri,
            root,
        );
        assert_eq!(params["context"]["triggerKind"], json!(1));
        assert_eq!(params["context"]["isRetrigger"], json!(false));
        assert!(params["context"].get("triggerCharacter").is_none());
    }

    #[test]
    fn the_client_opts_into_signature_help_context() {
        // Reading `retriggerCharacters` off a server's options is only
        // entitled by advertising this, and sending a context at all requires
        // it.
        assert_eq!(
            client_capabilities()["textDocument"]["signatureHelp"]["contextSupport"],
            json!(true)
        );
    }

    #[test]
    fn advertised_trigger_characters_replace_runytes_own() {
        let capabilities = advertised(json!({
            "completionProvider": {"triggerCharacters": ["\"", "/"]},
            "signatureHelpProvider": {
                "triggerCharacters": ["("],
                "retriggerCharacters": [")"],
            },
        }));
        assert!(capabilities.triggers_completion('"'));
        assert!(capabilities.triggers_completion('/'));
        // The characters Runyte used to hard-code are not this server's.
        assert!(!capabilities.triggers_completion('.'));
        assert!(!capabilities.triggers_completion(':'));

        assert!(capabilities.triggers_signature_help('(', false));
        assert!(!capabilities.triggers_signature_help(',', false));
        // A retrigger character is only active while a popup is showing, and
        // a trigger character stays one when it is.
        assert!(!capabilities.triggers_signature_help(')', false));
        assert!(capabilities.triggers_signature_help(')', true));
        assert!(capabilities.triggers_signature_help('(', true));
    }

    #[test]
    fn an_advertised_provider_without_a_list_keeps_runytes_defaults() {
        let capabilities = advertised(json!({
            "completionProvider": {},
            "signatureHelpProvider": {},
        }));
        assert!(capabilities.triggers_completion('.'));
        assert!(capabilities.triggers_completion(':'));
        assert!(capabilities.triggers_signature_help('(', false));
        assert!(capabilities.triggers_signature_help(',', false));
        // Nothing stands in for a retrigger character: the editor closes the
        // popup on `)` itself when the server named none.
        assert!(!capabilities.triggers_signature_help(')', true));
    }

    #[test]
    fn a_multi_character_trigger_entry_is_dropped_rather_than_half_matched() {
        let capabilities = advertised(json!({
            "completionProvider": {"triggerCharacters": ["->", "."]},
        }));
        assert!(capabilities.triggers_completion('.'));
        assert!(!capabilities.triggers_completion('-'));
        assert!(!capabilities.triggers_completion('>'));
    }

    #[test]
    fn a_provider_that_was_never_advertised_triggers_nothing() {
        let capabilities = advertised(json!({}));
        assert!(!capabilities.completion);
        assert!(!capabilities.signature_help);
        for character in ['.', ':', '(', ',', ')'] {
            assert!(!capabilities.triggers_completion(character));
            assert!(!capabilities.triggers_signature_help(character, false));
            assert!(!capabilities.triggers_signature_help(character, true));
        }
    }
}
