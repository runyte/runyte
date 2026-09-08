// SPDX-License-Identifier: MPL-2.0

//! XML, HCL/Terraform, Ruby, and PHP highlighting through the public syntax API.

use std::path::Path;

use runyte::{
    syntax::{DocumentSyntax, Registry, Span},
    text::{Text, Transaction},
};

fn parse(source: &str, language: &str) -> (Registry, Text, DocumentSyntax) {
    let registry = Registry::new();
    let language = registry
        .language_for_name(language)
        .expect("bundled language");
    let text = Text::from_str(source);
    let syntax =
        DocumentSyntax::new(&text, language, &registry).expect("usable grammar and queries");
    assert!(registry.errors().is_empty(), "{:?}", registry.errors());
    (registry, text, syntax)
}

fn spans(source: &str, language: &str) -> Vec<Span> {
    let (registry, text, syntax) = parse(source, language);
    syntax.spans(&text, &registry, 0, text.len_chars())
}

fn assert_scope(source: &str, spans: &[Span], needle: &str, expected: &str) {
    let byte = source.find(needle).expect("fixture token");
    let start = source[..byte].chars().count();
    let end = start + needle.chars().count();
    assert!(
        spans
            .iter()
            .any(|span| span.from <= start && span.to >= end && span.scope.name() == expected),
        "expected {needle:?} to have scope {expected:?}; got {:?}",
        spans
            .iter()
            .map(|span| (
                source
                    .chars()
                    .skip(span.from)
                    .take(span.to - span.from)
                    .collect::<String>(),
                span.scope.name()
            ))
            .collect::<Vec<_>>()
    );
}

#[test]
fn new_language_detection_covers_extensions_special_names_and_shebangs() {
    let registry = Registry::new();
    for (name, paths) in [
        (
            "xml",
            &[
                "note.XML",
                "logo.svg",
                "schema.xsd",
                "style.xsl",
                "style.xslt",
                "api.wsdl",
                "view.xaml",
                "App.csproj",
                "App.fsproj",
                "App.vbproj",
                "build.props",
                "build.targets",
                "strings.resx",
                "Info.plist",
                "pom.xml",
                "AndroidManifest.xml",
            ][..],
        ),
        (
            "hcl",
            &[
                "main.tf",
                "terraform.tfvars",
                "production.auto.tfvars",
                "terragrunt.hcl",
                "main.TF",
            ][..],
        ),
        (
            "ruby",
            &[
                "main.RB",
                "tasks.rake",
                "app.gemspec",
                "config.ru",
                "Gemfile",
                "Rakefile",
                "Guardfile",
                "Vagrantfile",
                "Brewfile",
                "Podfile",
                "Fastfile",
                "Appfile",
                ".irbrc",
                ".pryrc",
            ][..],
        ),
        (
            "php",
            &[
                "index.PHP",
                "view.phtml",
                "legacy.php3",
                "legacy.php4",
                "legacy.php5",
                "app.php7",
                "app.php8",
                "example.phps",
            ][..],
        ),
    ] {
        let language = registry.language_for_name(name).unwrap();
        for path in paths {
            assert_eq!(
                registry.language_for_path(Path::new(path)),
                Some(language),
                "{path}"
            );
        }
    }
    for (source, name) in [
        ("#!/usr/bin/env ruby\n", "ruby"),
        ("#!/usr/bin/jruby\n", "ruby"),
        ("#!/usr/bin/env -S php -n\n", "php"),
    ] {
        assert_eq!(
            registry.language_for_document(None, &Text::from_str(source)),
            registry.language_for_name(name)
        );
    }
    for path in [
        "Gemfile.lock",
        "composer.lock",
        "header.inc",
        "template.erb",
    ] {
        assert!(
            registry.language_for_path(Path::new(path)).is_none(),
            "{path}"
        );
    }
    assert_eq!(
        registry.language_for_path(Path::new("main.tf.json")),
        registry.language_for_name("json")
    );
    for (name, marker) in [
        ("xml", None),
        ("hcl", Some("#")),
        ("ruby", Some("#")),
        ("php", Some("//")),
    ] {
        assert_eq!(
            registry.line_comment(registry.language_for_name(name).unwrap()),
            marker
        );
    }
}

#[test]
fn xml_highlights_names_attributes_comments_entities_and_cdata() {
    let source = "<?xml version=\"1.0\"?>\n<!DOCTYPE note [<!ELEMENT note ANY>]>\n<!-- café -->\n<note role=\"世界\">&amp;<![CDATA[raw <text>]]><child /></note>\n";
    let highlighted = spans(source, "xml");
    for (needle, scope) in [
        ("xml", "keyword"),
        ("DOCTYPE", "keyword"),
        ("ELEMENT", "keyword"),
        ("<!-- café -->", "comment"),
        ("role", "property"),
        ("世界", "string"),
        ("&amp;", "constant"),
        ("raw <text>", "markup.raw"),
        ("child", "tag"),
    ] {
        assert_scope(source, &highlighted, needle, scope);
    }
}

#[test]
fn hcl_highlights_blocks_expressions_templates_and_all_comment_styles() {
    let source = r#"# infrastructure
// deployment
/* settings */
resource "aws_instance" "web" {
  count = 2
  enabled = true
  fallback = null
  ami = var.image_id
  name = "hello ${upper(var.name)}"
  tags = { Owner = "世界" }
  ports = [for port in var.ports : port if port > 0]
  message = <<-EOF
hello ${var.name}
EOF
  condition = "%{ if var.enabled }yes%{ else }no%{ endif }"
}
"#;
    let highlighted = spans(source, "hcl");
    for (needle, scope) in [
        ("# infrastructure", "comment"),
        ("// deployment", "comment"),
        ("/* settings */", "comment"),
        ("resource", "keyword"),
        ("aws_instance", "string"),
        ("count", "property"),
        ("2", "number"),
        ("true", "constant"),
        ("null", "constant"),
        ("var", "variable"),
        ("image_id", "property"),
        ("upper", "function"),
        ("Owner", "property"),
        ("世界", "string"),
        ("for", "keyword"),
        ("EOF", "label"),
        ("endif", "keyword"),
    ] {
        assert_scope(source, &highlighted, needle, scope);
    }
}

#[test]
fn ruby_highlights_definitions_calls_interpolation_and_local_variables() {
    let source = "# café\nclass Greeter\n  def greet(name)\n    message = \"hello #{name}, 世界\"\n    puts message\n    /hello/ =~ message\n    :ready\n  end\nend\n";
    let highlighted = spans(source, "ruby");
    for (needle, scope) in [
        ("# café", "comment"),
        ("class", "keyword"),
        ("Greeter", "constructor"),
        ("def", "keyword"),
        ("greet", "function"),
        ("name", "variable"),
        ("message", "variable"),
        ("puts", "function"),
        ("世界", "string"),
        ("/hello/", "string"),
        (":ready", "string"),
    ] {
        assert_scope(source, &highlighted, needle, scope);
    }
    let reference = source.rfind("message").unwrap();
    let offset = source[..reference].chars().count();
    assert!(
        highlighted
            .iter()
            .any(|span| span.from <= offset && span.to > offset && span.scope.name() == "variable")
    );
}

#[test]
fn ruby_local_names_do_not_override_explicit_methods_or_escape_method_scope() {
    let source = "def greet(name)\n  puts name\n  object.name\nend\nname\n";
    let highlighted = spans(source, "ruby");
    for (needle, skip, expected) in [
        ("puts name", 5, "variable"),
        ("object.name", 7, "function"),
        ("end\nname", 4, "function"),
    ] {
        let offset = source.find(needle).unwrap() + skip;
        assert!(
            highlighted.iter().any(|span| span.from <= offset
                && span.to >= offset + 4
                && span.scope.name() == expected),
            "{needle}: expected {expected}, got {highlighted:?}"
        );
    }
}

#[test]
fn ruby_class_module_and_singleton_scopes_keep_locals_isolated() {
    for declaration in [
        "class Example",
        "module Example",
        "class << self",
        "def self.example",
    ] {
        let source =
            format!("outer = 1\n{declaration}\n  outer\n  inner = 2\n  inner\nend\ninner\n");
        let highlighted = spans(&source, "ruby");
        for (offset, scope) in [
            (source.find("  outer").unwrap() + 2, "function"),
            (source.find("  inner\n").unwrap() + 2, "variable"),
            (source.rfind("inner").unwrap(), "function"),
        ] {
            assert!(
                highlighted.iter().any(|span| span.from <= offset
                    && span.to >= offset + 5
                    && span.scope.name() == scope),
                "{declaration}: {offset} should be {scope}: {highlighted:?}"
            );
        }
    }
    let source = "def self.greet(name)\n  puts name\nend\nname\n";
    let highlighted = spans(source, "ruby");
    let reference = source.rfind("name").unwrap();
    assert!(highlighted.iter().any(|span| span.from <= reference
        && span.to >= reference + 4
        && span.scope.name() == "function"));
}

#[test]
fn ruby_for_loop_bindings_remain_local_after_the_loop() {
    for pattern in ["item", "item, other", "item, *other"] {
        let source = format!("for {pattern} in [[1, 2]]\n  puts item\nend\nitem\n");
        let highlighted = spans(&source, "ruby");
        for (offset, _) in source.match_indices("item") {
            assert!(
                highlighted.iter().any(|span| span.from <= offset
                    && span.to >= offset + 4
                    && span.scope.name() == "variable"),
                "{pattern}: {offset} should be variable: {highlighted:?}"
            );
        }
    }
}

#[test]
fn ruby_class_headers_still_resolve_enclosing_locals() {
    for (source, needle) in [
        ("base = Class.new\nclass Example < base\nend\n", "base"),
        ("object = Object.new\nclass << object\nend\n", "object"),
        (
            "namespace = Module.new\nmodule namespace::Example\nend\n",
            "namespace",
        ),
    ] {
        let highlighted = spans(source, "ruby");
        let offset = source.rfind(needle).unwrap();
        assert!(
            highlighted.iter().any(|span| span.from <= offset
                && span.to >= offset + needle.len()
                && span.scope.name() == "variable"),
            "{source}: {highlighted:?}"
        );
    }
}

#[test]
fn ruby_singleton_receiver_highlighting_has_a_documented_scope_limit() {
    let source = "object = Object.new\ndef object.greet\nend\n";
    let highlighted = spans(source, "ruby");
    let receiver = source.rfind("object").unwrap();
    assert!(
        highlighted.iter().any(|span| span.from <= receiver
            && span.to >= receiver + 6
            && span.scope.name() == "function"),
        "whole-method scopes cannot resolve the receiver's outer local: {highlighted:?}"
    );
}

#[test]
fn ruby_exception_and_pattern_bindings_are_local_variables() {
    for source in [
        "begin\n  risky\nrescue => item\n  puts item\nend\n",
        "case value\nin item\n  puts item\nend\n",
        "case value\nin [item]\n  puts item\nend\n",
        "case value\nin [{name: [item]}]\n  puts item\nend\n",
        "case value\nin [*before, item, *after]\n  puts item\nend\n",
        "case value\nin (item)\n  puts item\nend\n",
        "case value\nin Integer => item\n  puts item\nend\n",
        "value => item\nputs item\n",
        "value in item\nputs item\n",
    ] {
        let highlighted = spans(source, "ruby");
        for (offset, _) in source.match_indices("item") {
            assert!(
                highlighted.iter().any(|span| span.from <= offset
                    && span.to >= offset + 4
                    && span.scope.name() == "variable"),
                "{source}: {offset}: {highlighted:?}"
            );
        }
    }
    let source = "case value\nin {item:}\n  puts item\nend\n";
    let highlighted = spans(source, "ruby");
    let offset = source.rfind("item").unwrap();
    assert!(highlighted.iter().any(|span| span.from <= offset
        && span.to >= offset + 4
        && span.scope.name() == "variable"));
}

#[test]
fn php_highlights_modern_syntax_and_preserves_doc_comments() {
    let source = "<?php\n/** café */\nreadonly class Greeter {\n  public function greet(string $name): string {\n    $count = 42;\n    return match ($name) { '世界' => \"hello\", default => null };\n  }\n}\n";
    let highlighted = spans(source, "php");
    for (needle, scope) in [
        ("<?php", "tag"),
        ("/** café */", "comment"),
        ("readonly", "keyword"),
        ("class", "keyword"),
        ("public", "keyword"),
        ("function", "keyword"),
        ("greet", "function"),
        ("string", "type"),
        ("name", "variable"),
        ("42", "number"),
        ("return", "keyword"),
        ("match", "keyword"),
        ("世界", "string"),
        ("null", "constant"),
    ] {
        assert_scope(source, &highlighted, needle, scope);
    }
}

#[test]
fn php_templates_highlight_html_css_and_javascript_across_php_regions() {
    let source = "<section class=\"card\"><?php echo $title; ?><span>世界</span></section>\n<style>body { color: red; }</style>\n<script>const answer = 42;</script>\n";
    let highlighted = spans(source, "php");
    for (needle, scope) in [
        ("section", "tag"),
        ("class", "attribute"),
        ("card", "string"),
        ("echo", "keyword"),
        ("title", "variable"),
        ("span", "tag"),
        ("color", "property"),
        ("const", "keyword"),
        ("42", "number"),
    ] {
        assert_scope(source, &highlighted, needle, scope);
    }
}

#[test]
fn php_heredocs_resolve_bundled_languages_and_keep_unknown_bodies_readable() {
    let source = "<?php\n$query = <<<'SQL'\nSELECT 7;\nSQL;\n$data = <<<JSON\n{\"answer\": 42}\nJSON;\n$plain = <<<TEXT\nhello 世界\nTEXT;\n";
    let highlighted = spans(source, "php");
    for (needle, scope) in [
        ("SELECT", "keyword"),
        ("42", "number"),
        ("hello 世界", "string"),
    ] {
        assert_scope(source, &highlighted, needle, scope);
    }
}

#[test]
fn php_injected_heredoc_languages_control_interpolation_colours() {
    for (source, interpolation, keyword) in [
        (
            "<?php\n$data = <<<SQL\nSELECT * FROM users WHERE id = $id;\nSQL;\n",
            "$id",
            "SELECT",
        ),
        (
            "<?php\n$data = <<<JSON\n{\"name\": \"$name\", \"n\": 42}\nJSON;\n",
            "$name",
            "42",
        ),
    ] {
        let highlighted = spans(source, "php");
        assert_scope(source, &highlighted, interpolation, "string");
        assert_scope(
            source,
            &highlighted,
            keyword,
            if keyword == "42" { "number" } else { "keyword" },
        );
    }
}

#[test]
fn new_languages_highlight_inside_markdown_fences() {
    for (marker, code, needle, scope) in [
        ("xml", "<note role=\"café\" />", "note", "tag"),
        ("hcl", "enabled = true", "true", "constant"),
        (
            "tf",
            "resource \"test\" \"example\" {}",
            "resource",
            "keyword",
        ),
        (
            "ruby",
            "def greet(name)\n  puts name\nend",
            "greet",
            "function",
        ),
        ("php", "<?php echo \"世界\"; ?>", "echo", "keyword"),
    ] {
        let source = format!("# Sample\n\n```{marker}\n{code}\n```\n");
        assert_scope(&source, &spans(&source, "markdown"), needle, scope);
    }
}

#[test]
fn new_languages_incrementally_recover_from_incomplete_unicode_source() {
    for (language, source, completion) in [
        (
            "xml",
            "<!-- café -->\n<note role=\"",
            "世界\">hello</note>\n",
        ),
        (
            "hcl",
            "# café\nresource \"demo\" \"main\" { name = \"",
            "世界\"\n}\n",
        ),
        (
            "ruby",
            "# café\ndef greet(name)\n  puts \"",
            "世界 #{name}\"\nend\n",
        ),
        (
            "php",
            "<?php // café\nfunction greet($name) { echo \"",
            "世界 $name\"; }\n?>\n<div>done</div>\n",
        ),
    ] {
        let (registry, mut text, mut syntax) = parse(source, language);
        let before = text.clone();
        let transaction = Transaction::insert(text.len_chars(), completion);
        text.apply(&transaction);
        assert!(syntax.update(&before, &text, &transaction, &registry));
        let fresh = DocumentSyntax::new(&text, syntax.language(), &registry).unwrap();
        assert_eq!(
            syntax.spans(&text, &registry, 0, text.len_chars()),
            fresh.spans(&text, &registry, 0, text.len_chars()),
            "{language}"
        );
    }
}

#[test]
fn large_php_keeps_php_highlights_without_html_injections() {
    let source = format!(
        "<?php\n{}echo 42;\n?>\n<div>done</div>\n",
        "// padding\n".repeat(14_000)
    );
    assert!(source.len() > 128 * 1024);
    let highlighted = spans(&source, "php");
    assert_scope(&source, &highlighted, "echo", "keyword");
    assert_scope(&source, &highlighted, "42", "number");
    let offset = source.find("div").unwrap();
    assert!(
        !highlighted
            .iter()
            .any(|span| span.from <= offset && span.to > offset && span.scope.name() == "tag")
    );
}
