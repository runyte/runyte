// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn markdown_reflow_keeps_shorter_nested_fences_inside_code() {
    for marker in ['`', '~'] {
        let outer = marker.to_string().repeat(4);
        let inner = marker.to_string().repeat(3);
        let source = format!(
            "{outer}markdown\n{inner}rust\nlet first_statement = 1; let second_statement = 2;\n{inner}\n{outer}\n"
        );
        assert_eq!(reflow(&source, 12, ReflowKind::Markdown), source);
    }
}

#[test]
fn markdown_reflow_requires_a_bare_closing_fence() {
    for marker in ['`', '~'] {
        let fence = marker.to_string().repeat(3);
        let source = format!(
            "{fence}\n{fence}still code\nlet first_statement = 1; let second_statement = 2;\n{fence}\n"
        );
        assert_eq!(reflow(&source, 12, ReflowKind::Markdown), source);
    }
}

#[test]
fn markdown_reflow_resumes_prose_after_a_longer_closing_fence() {
    for marker in ['`', '~'] {
        let open = marker.to_string().repeat(3);
        let close = marker.to_string().repeat(4);
        let code = "let first_statement = 1; let second_statement = 2;";
        let source = format!("{open}rust\n{code}\n{close} \t\nalpha beta gamma delta\n");
        let expected = format!("{open}rust\n{code}\n{close} \t\nalpha beta\ngamma delta\n");
        assert_eq!(reflow(&source, 12, ReflowKind::Markdown), expected);
    }
}
