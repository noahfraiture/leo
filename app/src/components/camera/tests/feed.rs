use super::READER_PROGRAM;

#[test]
fn reader_program_does_not_send_credentials() {
    assert!(!READER_PROGRAM.contains("user:"));
    assert!(!READER_PROGRAM.contains("pass:"));
}
