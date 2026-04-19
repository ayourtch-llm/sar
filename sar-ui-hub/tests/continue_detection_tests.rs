use sar_ui_hub::is_continue_message;

#[test]
fn test_continue_with_reason() {
    assert_eq!(
        is_continue_message("/continue interrupted the sleep"),
        Some("interrupted the sleep".to_string())
    );
}

#[test]
fn test_continue_with_empty_reason() {
    assert_eq!(
        is_continue_message("/continue "),
        Some("".to_string())
    );
}

#[test]
fn test_continue_without_reason() {
    assert_eq!(
        is_continue_message("/continue"),
        Some("".to_string())
    );
}

#[test]
fn test_continue_with_multiple_words() {
    assert_eq!(
        is_continue_message("/continue this is a longer reason"),
        Some("this is a longer reason".to_string())
    );
}

#[test]
fn test_continue_with_leading_spaces() {
    assert_eq!(
        is_continue_message("/continue  two spaces"),
        Some(" two spaces".to_string())
    );
}

#[test]
fn test_non_continue_messages() {
    assert_eq!(is_continue_message("hello world"), None);
    assert_eq!(is_continue_message("/cancel something"), None);
    assert_eq!(is_continue_message("continue something"), None);
    assert_eq!(is_continue_message(""), None);
}