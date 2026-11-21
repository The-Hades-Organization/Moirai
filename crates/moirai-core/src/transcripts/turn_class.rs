//! Acompanhamento × decisão: classifica o turno do orquestrador pelo que ele fez, sem olhar o resultado
//! (`src/lib/turn-class.ts`). O que não se reconhece com certeza cai em decisão.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnKind {
    Follow,
    Decision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnTool {
    pub name: String,
    /// Texto do comando, só no Bash.
    pub command: Option<String>,
}

impl TurnTool {
    pub fn named(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            command: None,
        }
    }

    pub fn bash(command: &str) -> Self {
        Self {
            name: "Bash".to_owned(),
            command: Some(command.to_owned()),
        }
    }
}

fn rx(pattern: &str) -> Regex {
    Regex::new(pattern).expect("regex fixa válida")
}

static HADES_FOLLOW: LazyLock<Regex> =
    LazyLock::new(|| rx(r"^(wait|watch|agent\s+(list|ls|status)|gate-lock\s+status)(\s|$)"));
static HERDR_FOLLOW: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"^(pane\s+(read|list|get)|agent\s+(list|read|get|status)|workspace\s+list|tab\s+list)(\s|$)",
    )
});
static PIPE_FILTER: LazyLock<Regex> =
    LazyLock::new(|| rx(r"^(head|tail|grep|egrep|wc|sort|uniq|cut|tr)(\s|$)"));
static CHAIN_SPLIT: LazyLock<Regex> = LazyLock::new(|| rx(r"&&|\|\||[;\n]"));
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| rx(r"\s+"));
static TASK_OUTPUT: LazyLock<Regex> = LazyLock::new(|| rx(r"/tasks/[A-Za-z0-9_-]+\.output$"));
static SAFE_DEVNULL: LazyLock<Regex> = LazyLock::new(|| rx(r"\d?&?>\s*/dev/null"));
static SAFE_FD: LazyLock<Regex> = LazyLock::new(|| rx(r"\d?>&\d"));

const WAKE_TAG: &str = "<task-notification";

/// Texto de usuário que é o aviso de um comando em segundo plano (`isWakeText`, `turn-class.ts:32`).
pub fn is_wake_text(text: &str) -> bool {
    text.trim_start().starts_with(WAKE_TAG)
}

struct Segment {
    text: String,
    /// Vem depois de um `|`: recebe a saída do trecho anterior.
    piped: bool,
}

/// Trechos entre `&&`, `||`, `;` e quebra de linha, e as etapas de cada pipeline (`segments`, `turn-class.ts:47`).
fn segments(command: &str) -> Vec<Segment> {
    CHAIN_SPLIT
        .split(command)
        .flat_map(|chain| {
            chain
                .split('|')
                .enumerate()
                .map(|(index, text)| Segment {
                    text: text.trim().to_owned(),
                    piped: index > 0,
                })
                .filter(|part| !part.text.is_empty())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn strip_once(text: &str, pattern: &Regex) -> String {
    pattern.replace(text, "").into_owned()
}

static PREFIX_RTK: LazyLock<Regex> =
    LazyLock::new(|| rx(r"^(rtk(\s+proxy)?|time|command|exec)\s+"));
static PREFIX_TIMEOUT: LazyLock<Regex> = LazyLock::new(|| rx(r"^timeout\s+\S+\s+"));
static PREFIX_EXPORT: LazyLock<Regex> = LazyLock::new(|| rx(r"^export\s+\S+=\S*\s*"));
static PREFIX_ASSIGN: LazyLock<Regex> = LazyLock::new(|| rx(r"^[A-Za-z_][A-Za-z0-9_]*=\S*\s*"));
static CD_ONLY: LazyLock<Regex> = LazyLock::new(|| rx(r"^cd(\s|$)"));

/// Sem o que antecede o programa: `cd dir`, atribuição, `export`, `rtk [proxy]`, `time`, `timeout N` (`stripPrefix`,
/// `turn-class.ts:62`).
fn strip_prefix(segment: &str) -> Option<String> {
    let mut rest = segment.to_owned();
    loop {
        let next = strip_once(&rest, &PREFIX_RTK);
        let next = strip_once(&next, &PREFIX_TIMEOUT);
        let next = strip_once(&next, &PREFIX_EXPORT);
        let next = strip_once(&next, &PREFIX_ASSIGN);
        if next == rest {
            break;
        }
        rest = next;
    }
    if rest.is_empty() || CD_ONLY.is_match(&rest) {
        return None;
    }
    Some(rest)
}

/// Redireção que grava arquivo: `>` ou `>>` que não vai para /dev/null nem para outro descritor (`writesFile`,
/// `turn-class.ts:80`).
fn writes_file(segment: &str) -> bool {
    let without = SAFE_DEVNULL.replace_all(segment, "");
    let without = SAFE_FD.replace_all(&without, "");
    without.contains('>') || without.contains("<<")
}

/// `cat`/`tail`/`head` só sobre arquivos de saída de tarefa em segundo plano (`readsTaskOutput`, `turn-class.ts:92`).
fn reads_task_output(program: &str, words: &[&str]) -> bool {
    if !matches!(program, "cat" | "tail" | "head") {
        return false;
    }
    let files: Vec<&&str> = words
        .iter()
        .filter(|w| !w.starts_with('-') && (w.is_empty() || !w.bytes().all(|b| b.is_ascii_digit())))
        .collect();
    !files.is_empty() && files.iter().all(|f| TASK_OUTPUT.is_match(f))
}

fn program_ends_with(program: &str, name: &str, with_sh: bool) -> bool {
    let base = program.rsplit('/').next().unwrap_or(program);
    // o TS casa `(^|\/)nome(\.sh)?$`: o último trecho depois de `/` é o nome inteiro
    base == name || (with_sh && base == format!("{name}.sh"))
}

fn is_variable_program(program: &str) -> bool {
    static VAR: LazyLock<Regex> = LazyLock::new(|| rx(r"^\$\{?[A-Za-z_]+\}?(/hades)?$"));
    VAR.is_match(program)
}

/// O programa do segmento: `hades`, caminho que termina em `hades`/`hades-agent.sh`/`hades-watch.sh` ou variável
/// (`followsUp`, `turn-class.ts:100`).
fn follows_up(segment: &str) -> bool {
    let Some(body) = strip_prefix(segment) else {
        return false;
    };
    if writes_file(&body) {
        return false;
    }
    let mut words = WHITESPACE.split(&body);
    let program = words.next().unwrap_or("");
    let rest_words: Vec<&str> = words.collect();
    let rest = rest_words.join(" ");
    if reads_task_output(program, &rest_words) {
        return true;
    }
    if program_ends_with(program, "hades-watch", true) {
        return true;
    }
    if program_ends_with(program, "hades-agent", true) {
        static LIST: LazyLock<Regex> = LazyLock::new(|| rx(r"^(list|ls|status)(\s|$)"));
        return LIST.is_match(&rest);
    }
    if program_ends_with(program, "hades", false) || is_variable_program(program) {
        return HADES_FOLLOW.is_match(&rest);
    }
    if program_ends_with(program, "herdr", false) {
        return HERDR_FOLLOW.is_match(&rest);
    }
    false
}

/// O comando Bash inteiro é acompanhamento: todo trecho de programa é de acompanhamento, e `cd` sozinho não conta
/// (`bashIsFollowUp`, `turn-class.ts:116`).
pub fn bash_is_follow_up(command: &str) -> bool {
    let programs: Vec<Segment> = segments(command)
        .into_iter()
        .filter(|part| strip_prefix(&part.text).is_some())
        .collect();
    let has_first = programs.iter().any(|part| !part.piped);
    has_first
        && programs.iter().all(|part| {
            (part.piped && PIPE_FILTER.is_match(&strip_prefix(&part.text).unwrap_or_default()))
                || follows_up(&part.text)
        })
}

fn tool_follows_up(tool: &TurnTool) -> bool {
    if matches!(tool.name.as_str(), "Monitor" | "ScheduleWakeup") {
        return true;
    }
    tool.name == "Bash" && tool.command.as_deref().is_some_and(bash_is_follow_up)
}

/// Sem tool: só o despertar de um aviso vira acompanhamento; texto solto é resposta ao dono, decisão. Com tool: todas
/// de acompanhamento (`classifyTurn`, `turn-class.ts:135`).
pub fn classify_turn(tools: &[TurnTool], after_wake: bool) -> TurnKind {
    if tools.is_empty() {
        return if after_wake {
            TurnKind::Follow
        } else {
            TurnKind::Decision
        };
    }
    if tools.iter().all(tool_follows_up) {
        TurnKind::Follow
    } else {
        TurnKind::Decision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn follow(tools: &[TurnTool], after_wake: bool) -> bool {
        classify_turn(tools, after_wake) == TurnKind::Follow
    }

    #[test]
    fn bash_de_acompanhamento() {
        for command in [
            "hades wait agents --any",
            "~/.claude/skills/hades/loop/bin/hades watch --next",
            "cd ~/proj && hades agent status --compact",
            "rtk hades agent list",
            "herdr pane read 3 --lines 40",
            "H=~/.claude/skills/hades/loop/bin/hades; $H agent status orq",
            "$B/hades-agent.sh list",
            "tail -5 /tmp/claude-1000/-home-x/abc-123/tasks/bmlejcfn5.output",
            "hades wait agents --any 2>&1",
            "hades agent list | head -5",
        ] {
            assert!(bash_is_follow_up(command), "{command}");
        }
    }

    #[test]
    fn bash_de_decisao() {
        for command in [
            "hades agent prompt orq 'faça X'",
            "hades agent spawn x",
            "herdr agent prompt 3 'x'",
            "git commit -m x",
            "hades wait agents --any > /tmp/out.txt",
            "hades agent list && git push",
            "cat src/lib/usage.ts",
            "tail -5 /tmp/log.txt",
            "cat /tmp/claude-1000/p/s/tasks/a.output /etc/passwd",
            "cat <<EOF\nx\nEOF",
            "cd /tmp",
            "",
        ] {
            assert!(!bash_is_follow_up(command), "{command:?}");
        }
    }

    #[test]
    fn redirecao_para_dev_null_segue_acompanhamento() {
        assert!(bash_is_follow_up("hades agent status 2>/dev/null"));
        assert!(bash_is_follow_up("hades agent status &>/dev/null"));
    }

    #[test]
    fn turno_sem_tool_so_e_acompanhamento_apos_despertar() {
        assert!(follow(&[], true));
        assert!(!follow(&[], false));
    }

    #[test]
    fn monitor_e_schedule_wakeup_armam_a_espera() {
        assert!(follow(&[TurnTool::named("Monitor")], false));
        assert!(follow(&[TurnTool::named("ScheduleWakeup")], false));
    }

    #[test]
    fn wait_junto_de_escrita_no_mesmo_turno_e_decisao() {
        let wait = TurnTool::bash("hades wait agents --any");
        assert!(!follow(&[wait.clone(), TurnTool::named("Write")], false));
        assert!(!follow(
            &[wait, TurnTool::bash("hades agent prompt orq x")],
            false
        ));
    }

    #[test]
    fn varios_status_em_sequencia_sao_acompanhamento() {
        assert!(follow(
            &[
                TurnTool::bash("hades agent status a"),
                TurnTool::bash("herdr pane read 1")
            ],
            false
        ));
    }

    #[test]
    fn edit_agent_sendmessage_read_sao_decisao() {
        for name in ["Edit", "Agent", "SendMessage", "Read", "AskUserQuestion"] {
            assert!(!follow(&[TurnTool::named(name)], true), "{name}");
        }
    }

    #[test]
    fn bash_sem_comando_e_decisao() {
        assert!(!follow(&[TurnTool::named("Bash")], false));
    }

    #[test]
    fn despertar_so_vale_no_comeco_do_texto() {
        assert!(is_wake_text("<task-notification>\n<task-id>x</task-id>"));
        assert!(is_wake_text("  \n<task-notification>"));
        assert!(!is_wake_text("veja o <task-notification> no meio"));
    }
}
