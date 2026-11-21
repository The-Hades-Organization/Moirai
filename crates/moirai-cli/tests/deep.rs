//! `estimate --deep` ponta a ponta, com `hades` e provedor falsos.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

const PLAN: &str = "# P\n\n## Unidades\n\n| Unidade | Papel | Escopo |\n|---|---|---|\n| u1 | impl | 4 arquivos |\n| u2 | impl | 9 arquivos |\n";

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new(hades_body: Option<&str>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("plano.md"), PLAN).unwrap();
        script(
            &root.join("estimador"),
            &format!(
                "echo \"$@\" >> '{}/chamadas'\necho '{{\"lines\": 420}}'",
                root.display()
            ),
        );
        if let Some(body) = hades_body {
            script(&root.join("hades"), body);
        }
        Self { dir }
    }

    fn json_ok(&self) -> String {
        let root = self.dir.path().display();
        format!(
            "echo \"$@\" >> '{root}/pedidos'\necho '{{\"bin\":\"{root}/estimador\",\"args\":[\"-p\",\"prompt\"],\"cwd\":\"{root}\",\"timeout_s\":30}}'"
        )
    }

    fn run(&self, with_hades: bool) -> Output {
        let root = self.dir.path();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_moirai"));
        cmd.args(["estimate", "plano.md", "--deep", "--json"])
            .current_dir(root)
            .env_clear()
            .env("HOME", root)
            .env("PATH", "/usr/bin:/bin")
            .env("HADES_NOW", "2026-11-20T12:00:00Z");
        if with_hades {
            cmd.env("MOIRAI_HADES_BIN", root.join("hades"));
        }
        cmd.output().unwrap()
    }
}

#[test]
fn pede_o_argv_por_unidade_e_roda_o_estimador() {
    let sb = Sandbox::new(None);
    script(&sb.dir.path().join("hades"), &sb.json_ok());
    let out = sb.run(true);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let pedidos = fs::read_to_string(sb.dir.path().join("pedidos")).unwrap();
    assert_eq!(pedidos.matches("provider print-argv").count(), 2);
    assert!(pedidos.contains("provider print-argv --role ORCHESTRATOR --read-only --allow-tools Read Grep Glob --prompt Estime"));
    let chamadas = fs::read_to_string(sb.dir.path().join("chamadas")).unwrap();
    assert_eq!(chamadas.lines().count(), 2);
    assert!(!String::from_utf8_lossy(&out.stderr).contains("sem resposta utilizável"));
}

#[test]
fn sem_hades_no_path_sai_com_2() {
    let sb = Sandbox::new(None);
    let out = sb.run(false);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("`hades` não está no PATH"));
}

#[test]
fn hades_que_falha_sai_com_2_na_primeira_unidade() {
    let sb = Sandbox::new(Some("echo quebrou >&2\nexit 1"));
    let out = sb.run(true);
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("print-argv` falhou: saiu 1: quebrou"), "{err}");
}

#[test]
fn estimador_que_falha_so_avisa_e_segue() {
    let sb = Sandbox::new(None);
    let root = sb.dir.path().display();
    script(
        &sb.dir.path().join("hades"),
        &format!(
            "echo '{{\"bin\":\"/bin/false\",\"args\":[],\"cwd\":\"{root}\",\"timeout_s\":30}}'"
        ),
    );
    let out = sb.run(true);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr)
            .matches("sem resposta utilizável")
            .count(),
        2
    );
}
