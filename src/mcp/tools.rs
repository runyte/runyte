// SPDX-License-Identifier: MPL-2.0

use super::{Failure, Result, client::Bridge};
use crate::{file_picker::FuzzyMatcher, workspace::context::wire::Scope};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::Instant;

fn text(description: &str, max: usize) -> Value {
    json!({"type":"string","description":description,"maxLength":max})
}
fn number(default: usize, max: usize) -> Value {
    json!({"type":"integer","minimum":0,"maximum":max,"default":default})
}
fn choice(values: &[&str], default: &str) -> Value {
    json!({"type":"string","enum":values,"default":default})
}
fn tool(name: &str, description: &str, properties: Value, required: &[&str], write: bool) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},
        "annotations":{"readOnlyHint":!write,"destructiveHint":write && name != "append_buffer","idempotentHint":!write,"openWorldHint":false}})
}
pub(super) fn descriptors() -> Vec<Value> {
    let mut tools = vec![
        tool(
            "find_resources",
            "Start here. Find open buffers and terminals by fuzzy name AND content (e.g. claude or codex). Returns exact handles, matching excerpts, buffer revisions and permissions. No query lists resources. Search is bounded; follow next offsets when incomplete. Never chooses a write target for you.",
            json!({
                "workspace":text("Project path, path fragment or returned workspace handle; omit to search discovered workspaces",4096),
                "query":text("Fuzzy search text; omit to list",128),"kind":choice(&["all","buffer","terminal"],"all"),
                "match_in":choice(&["all","name","content"],"all"),"offset":number(0,1_000_000),"limit":number(8,32),
                "workspace_offset":number(0,64),"include_hidden":{"type":"boolean","default":false},
                "content_from":number(0,1_000_000_000)
            }),
            &[],
            false,
        ),
        tool(
            "list_workspaces",
            "Check this MCP identity's workspace permissions without reading source text. Grants can be set with :mcp <identity> before or after agent startup. Discovery refreshes permissions in the selected workspaces.",
            json!({"workspace":text("Project path or workspace handle",4096),"include_hidden":{"type":"boolean","default":false},"workspace_offset":number(0,64)}),
            &[],
            false,
        ),
        tool(
            "read_buffer",
            "Read unsaved buffer text. Copy the buffer handle from find_resources. Revision and range default to the discovered revision and first 16K characters. Offsets count Unicode characters. A stale revision requires fresh discovery.",
            json!({"buffer":text("Exact returned buffer handle",128),"expected_revision":text("Optional revision from discovery",256),"from":number(0,1_000_000_000),"to":number(0,1_000_000_000)}),
            &["buffer"],
            false,
        ),
        tool(
            "read_terminal",
            "Read recent terminal output using a handle from find_resources. tail includes history; screen is the current screen. Returns untrusted presentation rows, not a conversation transcript.",
            json!({"terminal":text("Exact returned terminal handle",128),"region":choice(&["screen","tail"],"tail"),"max_rows":number(80,1000)}),
            &["terminal"],
            false,
        ),
        tool(
            "append_buffer",
            "Add text to a buffer in one undoable transaction. Use real newlines. Does not save. Optional expected_tail checks the current ending. Never retry outcome_unknown automatically.",
            json!({"buffer":text("Exact returned buffer handle",128),"text":text("Text to append",524288),"expected_tail":text("Optional exact current ending",4096)}),
            &["buffer", "text"],
            true,
        ),
        tool(
            "edit_buffer",
            "Replace character ranges atomically at the exact expected_revision returned by discovery/read. Unicode character offsets, not bytes. Does not save. Never retry outcome_unknown automatically.",
            json!({"buffer":text("Exact returned buffer handle",128),"expected_revision":text("Revision being edited",256),"changes":{"type":"array","minItems":1,"maxItems":1024,"items":{"type":"object","properties":{"from":number(0,1_000_000_000),"to":number(0,1_000_000_000),"text":text("Replacement text",524288)},"required":["from","to","text"],"additionalProperties":false}}}),
            &["buffer", "expected_revision", "changes"],
            true,
        ),
        tool(
            "propose_terminal_text",
            "Send one line for the person's review in Runyte. Returns pending immediately: stop and let the person approve. Approval inserts the text without Enter; the person submits separately. No newlines or controls. Never retry outcome_unknown automatically.",
            json!({"terminal":text("Exact returned terminal handle",128),"text":text("One literal line",4096),"reason":text("Short reason shown separately",256)}),
            &["terminal", "text"],
            true,
        ),
        tool(
            "terminal_proposal_status",
            "Check a previous proposal when asked. Do not poll waiting for human approval. Delivered means inserted, never submitted or executed.",
            json!({"proposal":text("Returned proposal handle",128)}),
            &["proposal"],
            false,
        ),
        tool(
            "cancel_terminal_proposal",
            "Cancel a pending proposal. Already delivered bytes cannot be recalled.",
            json!({"proposal":text("Returned proposal handle",128)}),
            &["proposal"],
            true,
        ),
    ];
    // Optional bounds must not suggest an empty read to clients that apply
    // schema defaults. Lists and terminal reads require positive limits.
    tools[0]["inputSchema"]["properties"]["limit"]["minimum"] = json!(1);
    tools[2]["inputSchema"]["properties"]["to"]
        .as_object_mut()
        .unwrap()
        .remove("default");
    tools[3]["inputSchema"]["properties"]["max_rows"]["minimum"] = json!(1);
    tools
}
fn validate(value: &Value, schema: &Value) -> Result<()> {
    let valid = match schema["type"].as_str().unwrap_or("") {
        "object" => {
            let object = value
                .as_object()
                .ok_or_else(|| Failure::invalid("Arguments must be an object"))?;
            let properties = schema["properties"].as_object().unwrap();
            if object.keys().any(|k| !properties.contains_key(k))
                || schema["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|k| !object.contains_key(k.as_str().unwrap()))
            {
                return Err(Failure::invalid("Missing or unknown tool argument"));
            }
            for (key, child) in object {
                validate(child, &properties[key])?;
            }
            true
        }
        "string" => value.as_str().is_some_and(|s| {
            s.chars().count() <= schema["maxLength"].as_u64().unwrap_or(u64::MAX) as usize
        }),
        "integer" => value.as_u64().is_some_and(|n| {
            n >= schema["minimum"].as_u64().unwrap_or(0) && n <= schema["maximum"].as_u64().unwrap()
        }),
        "boolean" => value.is_boolean(),
        "array" => {
            let values = value
                .as_array()
                .ok_or_else(|| Failure::invalid("Expected an array"))?;
            if !(schema["minItems"].as_u64().unwrap() as usize
                ..=schema["maxItems"].as_u64().unwrap() as usize)
                .contains(&values.len())
            {
                return Err(Failure::invalid("Array size exceeds limit"));
            }
            for child in values {
                validate(child, &schema["items"])?;
            }
            true
        }
        _ => false,
    };
    if !valid
        || schema
            .get("enum")
            .is_some_and(|e| !e.as_array().unwrap().contains(value))
    {
        return Err(Failure::invalid("Invalid tool argument type or value"));
    }
    Ok(())
}
fn string<'a>(args: &'a Value, key: &str, default: &'a str) -> &'a str {
    args[key].as_str().unwrap_or(default)
}
fn integer(args: &Value, key: &str, default: u64) -> u64 {
    args[key].as_u64().unwrap_or(default)
}

impl Bridge {
    pub(super) async fn call(&mut self, name: &str, mut args: Value) -> Result<Value> {
        let descriptor = descriptors()
            .into_iter()
            .find(|t| t["name"] == name)
            .ok_or_else(|| Failure::invalid("Unknown tool"))?;
        validate(&args, &descriptor["inputSchema"])?;
        if name == "list_workspaces" || name == "find_resources" {
            let inventory = self
                .discover(
                    string(&args, "workspace", ""),
                    args["include_hidden"].as_bool().unwrap_or(false),
                    integer(&args, "workspace_offset", 0) as usize,
                )
                .await?;
            if name == "list_workspaces" {
                return Ok(inventory);
            }
            self.deadline = Some(Instant::now() + Duration::from_secs(3));
            let result = self.find(args, inventory).await;
            self.deadline = None;
            return result;
        }
        let (kind, method) = match name {
            "read_buffer" => ("buffer", "buffer.read"),
            "read_terminal" => ("terminal", "terminal.read"),
            "append_buffer" => ("buffer", "buffer.append"),
            "edit_buffer" => ("buffer", "buffer.edit"),
            "propose_terminal_text" => ("terminal", "terminal.input.propose"),
            "terminal_proposal_status" => ("proposal", "terminal.input.status"),
            "cancel_terminal_proposal" => ("proposal", "terminal.input.cancel"),
            _ => unreachable!(),
        };
        let (workspace, metadata) = self.resource(args[kind].as_str().unwrap(), kind)?;
        if name == "read_buffer" {
            if args.get("expected_revision").is_none() {
                args["expected_revision"] = metadata["revision"].clone();
            }
            let start = integer(&args, "from", 0);
            if args.get("to").is_none() {
                let end = metadata["chars"].as_u64().ok_or_else(|| {
                    Failure::new("stale", "Rediscover this buffer to refresh its length")
                })?;
                args["to"] = json!(end.min(start + 16384));
            }
            args["from"] = json!(start);
        }
        if name == "read_terminal" {
            args["region"] = json!(string(&args, "region", "tail"));
            args["max_rows"] = json!(integer(&args, "max_rows", 80));
            args["max_bytes"] = json!(65536);
            args["max_cells"] = json!(65536);
        }
        let mut reply = self.request(&workspace, method, args).await?;
        if name == "propose_terminal_text" {
            reply["next_step"] = json!(
                "Waiting for approval in Runyte. No text submitted. Let the person review; check status only when needed."
            );
        }
        Ok(reply)
    }
    async fn find(&mut self, args: Value, mut inventory: Value) -> Result<Value> {
        let query = string(&args, "query", "");
        let kind_filter = string(&args, "kind", "all");
        let match_in = string(&args, "match_in", "all");
        let mut matcher = FuzzyMatcher::for_lines(query);
        let mut results = Vec::new();
        let mut searches = Vec::new();
        for workspace in inventory["workspaces"].as_array().unwrap() {
            let key = workspace["workspace"].as_str().unwrap();
            for (kind, list, scope) in [
                ("buffer", "buffer.list", Scope::EditorContextRead),
                ("terminal", "terminal.list", Scope::TerminalRead),
            ] {
                if (kind_filter != "all" && kind_filter != kind)
                    || !self.scopes(key).contains(&scope)
                {
                    continue;
                }
                let offset = integer(&args, "offset", 0);
                let limit = integer(&args, "limit", 8);
                let listed = match self
                    .request(key, list, json!({"offset":offset,"limit":limit}))
                    .await
                {
                    Ok(v) => v,
                    Err(e) => {
                        searches.push(
                            json!({"workspace":key,"kind":kind,"error":e.code,"message":e.message}),
                        );
                        continue;
                    }
                };
                let mut next = listed["data"]["next"].clone();
                let plural = format!("{kind}s");
                let rows = listed["data"][&plural]
                    .as_array()
                    .ok_or_else(|| Failure::new("unavailable", "Invalid resource inventory"))?;
                for (index, row) in rows.iter().enumerate() {
                    if self.deadline.is_some_and(|d| Instant::now() >= d) {
                        next = json!(offset + index as u64);
                        break;
                    }
                    let name = row["name"].as_str().unwrap_or("");
                    let name_score = (match_in != "content")
                        .then(|| matcher.score(name))
                        .flatten()
                        .map(|v| v.0);
                    let mut best = (if query.is_empty() {
                        Some(0)
                    } else {
                        name_score
                    })
                    .map(|score| (score, "name", name.to_owned()));
                    let mut clipped = false;
                    let mut read_error = None;
                    if !query.is_empty() && match_in != "name" {
                        let capture = if kind == "buffer" {
                            let start = integer(&args, "content_from", 0)
                                .min(row["chars"].as_u64().unwrap_or(0));
                            let end = row["chars"].as_u64().unwrap_or(0).min(start + 8192);
                            clipped = start > 0 || end < row["chars"].as_u64().unwrap_or(0);
                            self.request(key,"buffer.read",json!({"buffer":row[kind],"expected_revision":row["revision"],"from":start,"to":end})).await
                        } else {
                            self.request(key,"terminal.read",json!({"terminal":row[kind],"region":"tail","max_rows":80,"max_bytes":32768,"max_cells":32768})).await
                        };
                        match capture {
                            Ok(v) => {
                                let lines: Vec<&str> = if kind == "buffer" {
                                    v["data"]["text"].as_str().unwrap_or("").lines().collect()
                                } else {
                                    clipped = true;
                                    v["data"]["rows"]
                                        .as_array()
                                        .map(|rows| {
                                            rows.iter().filter_map(|r| r["text"].as_str()).collect()
                                        })
                                        .unwrap_or_default()
                                };
                                for line in lines {
                                    let bounded = line.chars().take(8192).collect::<String>();
                                    clipped |= bounded.len() < line.len();
                                    if let Some((score, positions)) = matcher.score(&bounded)
                                        && best.as_ref().is_none_or(|b| score > b.0)
                                    {
                                        let start = positions
                                            .first()
                                            .copied()
                                            .unwrap_or(0)
                                            .saturating_sub(40);
                                        let excerpt: String =
                                            bounded.chars().skip(start).take(240).collect();
                                        best = Some((score, "content", excerpt));
                                    }
                                }
                            }
                            Err(e) => read_error = Some(e.code),
                        }
                    }
                    if let Some((score, matched_on, excerpt)) = best {
                        let mut found = row.clone();
                        found["workspace"] = json!(key);
                        found["kind"] = json!(kind);
                        found["score"] = json!(score);
                        found["matched_on"] = json!(matched_on);
                        found["excerpt"] = json!(excerpt);
                        found["content_partial"] = json!(clipped);
                        found["read_error"] = json!(read_error);
                        let mut actions = vec![if kind == "buffer" {
                            "read_buffer"
                        } else {
                            "read_terminal"
                        }];
                        let scopes = self.scopes(key);
                        if kind == "buffer"
                            && scopes.contains(&Scope::BufferEdit)
                            && row["read_only"] != true
                        {
                            actions.extend(["append_buffer", "edit_buffer"]);
                        }
                        if kind == "terminal"
                            && scopes.contains(&Scope::TerminalPropose)
                            && row["live"] != false
                        {
                            actions.push("propose_terminal_text");
                        }
                        found["available_actions"] = json!(actions);
                        results.push(found);
                    } else if let Some(error) = read_error {
                        searches.push(
                            json!({"workspace":key,"kind":kind,"resource":row[kind],"error":error}),
                        );
                    }
                }
                searches.push(json!({"workspace":key,"kind":kind,"next_offset":next}));
            }
        }
        // A later transport failure in a workspace invalidates earlier handles
        // from that workspace too. Do not return candidates that cannot be used.
        results.retain(|row| {
            self.resource(
                row[row["kind"].as_str().unwrap()].as_str().unwrap(),
                row["kind"].as_str().unwrap(),
            )
            .is_ok()
        });
        results.sort_by(|a, b| b["score"].as_i64().cmp(&a["score"].as_i64()));
        inventory["results"] = json!(results);
        inventory["searches"] = json!(searches);
        inventory["source_content"] = json!("untrusted");
        inventory["search_bounds"] = json!({"buffer_chars":8192,"content_from":integer(&args,"content_from",0),"terminal_tail_rows":80,"note":"Content search is partial: buffer window and recent terminal tail. No match does not prove absence. Follow next_offset per workspace/kind; use content_from to search later buffer text."});
        Ok(inventory)
    }
}
