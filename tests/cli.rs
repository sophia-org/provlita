//! The protected serve entry selects only the 9P file endpoint. These cases
//! stop before any socket, configuration or GPU access.
use std::process::Command;

fn serve(env: &[(&str, &str)]) -> (i32, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_provlita"))
        .arg("--serve")
        .env_clear()
        .envs(env.iter().copied())
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn retired_socket_variable_is_refused_even_empty_or_beside_the_file_endpoint() {
    for retired in ["/nonexistent/shell.sock", ""] {
        let (code, stderr) = serve(&[
            ("SOPHIA_SHELL_SOCKET", retired),
            ("SOPHIA_SHELL_9P_SOCKET", "/nonexistent/shell-9p.sock"),
        ]);
        assert_eq!(code, 2);
        assert!(
            stderr.contains("SOPHIA_SHELL_SOCKET is unsupported; use SOPHIA_SHELL_9P_SOCKET"),
            "{stderr}"
        );
    }
}

#[test]
fn file_endpoint_is_required_and_must_be_nonempty() {
    for env in [&[][..], &[("SOPHIA_SHELL_9P_SOCKET", "")][..]] {
        let (code, stderr) = serve(env);
        assert_eq!(code, 2);
        assert!(
            stderr.contains("SOPHIA_SHELL_9P_SOCKET is required"),
            "{stderr}"
        );
    }
}
