// SPDX-License-Identifier: MPL-2.0
use super::*;
use serde_json::json;

#[test]
fn typed_validation_result_round_trips_with_a_strict_discriminator() {
    let result = json!({"kind":"validation","surface":"u:1","revision":"i:2",
        "fields":[{"field":"name","status":"valid"}]});
    let parsed: ValidationResult = serde_json::from_value(result.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), result);
    let response = json!({"type":"response","id":"h:1","result":result});
    let decoded =
        crate::plugin::application::decode(&serde_json::to_vec(&response).unwrap()).unwrap();
    assert!(matches!(
        decoded,
        crate::plugin::ClientMessage::Application(
            crate::plugin::application::ClientMessage::Response {
                outcome: crate::plugin::application::CommandResponse::Validation { .. },
                ..
            }
        )
    ));
    for key in ["kind", "surface", "revision", "fields"] {
        let mut missing = result.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(serde_json::from_value::<ValidationResult>(missing).is_err());
    }
    for value in [
        json!({"kind":"other","surface":"u:1","revision":"i:2","fields":[]}),
        json!({"kind":"validation","surface":"u:1","revision":"i:2","fields":[],"message":"secret"}),
        json!({"kind":"validation","surface":"u:1","revision":"i:2","fields":[{"field":"name","status":"valid","message":"secret"}]}),
    ] {
        assert!(serde_json::from_value::<ValidationResult>(value).is_err());
    }
}

#[test]
fn optional_validation_fields_preserve_old_wire_and_bound_static_feedback() {
    let field = Field::text("name", "Name".into());
    let encoded = serde_json::to_value(&field).unwrap();
    assert!(encoded.get("validate").is_none());
    assert!(encoded.get("validation_message").is_none());
    let decoded: Field = serde_json::from_value(encoded).unwrap();
    assert!(!decoded.validate);
    assert!(decoded.validation_message.is_none());
    let mut field = field;
    field.kind = Kind::Secret;
    field.validate = true;
    field.validation_message = Some("Credentials rejected".into());
    validate("Connection", &[field.clone()]).unwrap();
    for message in ["x".repeat(161), "bad\u{85}".into(), String::new()] {
        field.validation_message = Some(message);
        assert!(validate("Connection", &[field.clone()]).is_err());
    }
}
