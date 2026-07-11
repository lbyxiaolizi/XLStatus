use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const TEST_SECRET: &str = "audit-secret-do-not-log";

fn server_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xlstatus-server"));
    command
        .env_clear()
        .env(
            "SESSION_SECRET",
            "integration-test-session-secret-with-at-least-32-bytes",
        )
        .env(
            "SECRET_ENCRYPTION_KEY",
            "integration-test-encryption-key-with-at-least-32-bytes",
        );
    command
}

fn run_with_database_url(database_url: &str) -> Output {
    server_command()
        .env("DATABASE_URL", database_url)
        .output()
        .expect("server process should start")
}

fn diagnostics(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_failed_without_database_secret(output: Output, expected_error: &str) {
    assert!(!output.status.success());
    let diagnostics = diagnostics(&output);
    assert!(diagnostics.contains(expected_error));
    assert!(!diagnostics.contains(TEST_SECRET));
    assert!(!diagnostics.contains("audit-user"));
}

fn temporary_config_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "xlstatus-startup-redaction-{}-{nonce}.toml",
        std::process::id()
    ))
}

#[test]
fn unsupported_database_url_is_redacted_from_startup_error() {
    let output = run_with_database_url(&format!(
        "mysql://audit-user:{TEST_SECRET}@db.example/xlstatus"
    ));

    assert_failed_without_database_secret(output, "Unsupported database URL");
}

#[test]
fn invalid_sqlite_url_is_redacted_from_startup_error() {
    let output = run_with_database_url(&format!(
        "sqlite://audit.db?mode=invalid&password={TEST_SECRET}"
    ));

    assert_failed_without_database_secret(output, "Invalid SQLite database URL");
}

#[test]
fn unknown_postgres_parameter_is_redacted_before_sqlx_logging() {
    let output = run_with_database_url(&format!(
        "postgresql://audit-user:password@127.0.0.1/xlstatus\
         ?sslpassword={TEST_SECRET}&sslmode=invalid"
    ));

    assert_failed_without_database_secret(output, "Invalid PostgreSQL database URL");
}

#[test]
fn malformed_pgpass_line_is_redacted_before_sqlx_logging() {
    let pgpass_path = temporary_config_path().with_extension("pgpass");
    fs::write(&pgpass_path, format!("*:*:{TEST_SECRET}\n"))
        .expect("temporary pgpass file should be writable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pgpass_path, fs::Permissions::from_mode(0o600))
            .expect("temporary pgpass permissions should be writable");
    }

    let output = server_command()
        .env(
            "DATABASE_URL",
            "postgresql://audit-user@127.0.0.1:1/xlstatus",
        )
        .env("PGPASSFILE", &pgpass_path)
        .output()
        .expect("server process should start");
    let _ = fs::remove_file(pgpass_path);

    assert_failed_without_database_secret(output, "Failed to connect to database");
}

#[test]
fn malformed_config_file_is_redacted_from_startup_error() {
    let config_path = temporary_config_path();
    let config =
        format!("[database]\nurl = \"postgresql://audit-user:{TEST_SECRET}@db.example/xlstatus\n");
    fs::write(&config_path, config).expect("temporary config should be writable");

    let output = server_command()
        .env("CONFIG_FILE", &config_path)
        .output()
        .expect("server process should start");
    let _ = fs::remove_file(config_path);

    assert_failed_without_database_secret(output, "Failed to parse configuration file");
}
