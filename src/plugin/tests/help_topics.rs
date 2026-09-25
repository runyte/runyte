// SPDX-License-Identifier: MPL-2.0
use super::*;

fn topic(id: &str) -> Topic {
    Topic {
        id: id.into(),
        title: "Rows".into(),
        paragraphs: vec!["Rows lists one page of a table.".into()],
    }
}

#[test]
fn accepts_bounded_distinct_topics() {
    assert_eq!(validate(&[topic("rows"), topic("record")]), Ok(()));
    assert_eq!(validate(&[]), Ok(()));
}

#[test]
fn refuses_duplicate_or_malformed_topics() {
    assert!(validate(&[topic("rows"), topic("rows")]).is_err());
    assert!(validate(&[topic("Rows")]).is_err());
    let mut untitled = topic("rows");
    untitled.title.clear();
    assert!(validate(&[untitled]).is_err());
    let mut long_title = topic("rows");
    long_title.title = "t".repeat(MAX_TITLE_BYTES + 1);
    assert!(validate(&[long_title]).is_err());
    let mut empty = topic("rows");
    empty.paragraphs.clear();
    assert!(validate(&[empty]).is_err());
    let mut blank = topic("rows");
    blank.paragraphs = vec![String::new()];
    assert!(validate(&[blank]).is_err());
    let mut control = topic("rows");
    control.paragraphs = vec!["one\ntwo".into()];
    assert!(validate(&[control]).is_err());
    let mut long = topic("rows");
    long.paragraphs = vec!["p".repeat(MAX_PARAGRAPH_BYTES + 1)];
    assert!(validate(&[long]).is_err());
}

#[test]
fn refuses_topic_and_total_limits() {
    let many = (0..=MAX_TOPICS)
        .map(|index| topic(&format!("t{index}")))
        .collect::<Vec<_>>();
    assert!(validate(&many).is_err());
    let heavy = (0..MAX_TOPICS)
        .map(|index| Topic {
            id: format!("t{index}"),
            title: "Heavy".into(),
            paragraphs: vec!["p".repeat(MAX_PARAGRAPH_BYTES); 3],
        })
        .collect::<Vec<_>>();
    assert_eq!(
        validate(&heavy),
        Err("Help topics exceed 64 KiB".to_owned())
    );
}

#[test]
fn decoding_stops_at_the_array_limits() {
    let paragraphs = vec!["p"; MAX_PARAGRAPHS + 1];
    let json = serde_json::json!({"id":"rows","title":"Rows","paragraphs":paragraphs});
    assert!(serde_json::from_value::<Topic>(json).is_err());
    let exact = vec!["p"; MAX_PARAGRAPHS];
    let json = serde_json::json!({"id":"rows","title":"Rows","paragraphs":exact});
    assert_eq!(
        serde_json::from_value::<Topic>(json)
            .unwrap()
            .paragraphs
            .len(),
        MAX_PARAGRAPHS
    );
    // The registration field streams the same way: the seventeenth topic
    // is refused before it is decoded.
    let topic = serde_json::json!({"id":"rows","title":"Rows","paragraphs":["p"]});
    let message = |count: usize| {
        serde_json::json!({
            "type":"register","version":"runyte-1","runyte":">=0.3.0","name":"Help",
            "commands":[],"required_features":[],"optional_features":[],
            "required_capabilities":[],"optional_capabilities":[],
            "help_topics":vec![topic.clone(); count]
        })
    };
    let decoded =
        |count| serde_json::from_value::<super::super::application::ClientMessage>(message(count));
    assert!(decoded(MAX_TOPICS + 1).is_err());
    let Ok(super::super::application::ClientMessage::Register {
        help_topics: Some(topics),
        ..
    }) = decoded(MAX_TOPICS)
    else {
        panic!("sixteen topics decode");
    };
    assert_eq!(topics.len(), MAX_TOPICS);
    let unknown = serde_json::json!({"id":"rows","title":"Rows","paragraphs":["p"],"keys":[]});
    assert!(serde_json::from_value::<Topic>(unknown).is_err());
}
