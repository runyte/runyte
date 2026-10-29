// SPDX-License-Identifier: MPL-2.0

use std::{env, fs, path::PathBuf};

fn main() {
    let directory = PathBuf::from("docs/user-guide");
    println!("cargo:rerun-if-changed=docs/user-guide");
    let index = fs::read_to_string(directory.join("README.md")).expect("user guide index");
    let chapters: Vec<_> = index
        .lines()
        .filter_map(|line| line.strip_prefix("- ["))
        .map(|line| {
            line.split_once("](")
                .expect("chapter link")
                .1
                .trim_end_matches(')')
        })
        .collect();
    let mut guide = String::from("# Runyte user guide\n\n");
    for chapter in &chapters {
        let source = fs::read_to_string(directory.join(chapter)).expect("user guide chapter");
        let mut body = source
            .split("<!-- navigation -->")
            .next()
            .unwrap()
            .trim_end()
            .to_owned();
        for target in &chapters {
            body = body.replace(&format!("]({target}#"), "](#");
        }
        body = body
            .replace("](../../", "](https://github.com/runyte/runyte/blob/main/")
            .replace(
                "](../",
                "](https://github.com/runyte/runyte/blob/main/docs/",
            );
        guide.push_str(&body);
        guide.push_str("\n\n");
    }
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("user-guide.md"),
        guide,
    )
    .expect("assembled user guide");
}
