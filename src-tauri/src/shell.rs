//! Shell command analysis for the risk engine.
//!
//! The policy engine must not be fooled by a harmless-looking prefix. Plain
//! prefix matching (`command.starts_with("ls")`) rated `ls && rm -rf ~` as a
//! read-only command, so an agent could smuggle destructive operations past
//! the guard rails. This module turns a command line into its simple commands,
//! unwraps interpreters and launchers (`sh -c`, `eval`, `xargs`, `find -exec`,
//! `sudo`, `timeout`, ...) and follows command substitutions (`$(...)`,
//! backticks) so the classifier can score every part and keep the worst.
//!
//! The analysis is deliberately conservative: constructs it cannot resolve are
//! reported as `dynamic`, and the policy engine treats those as "ask a human".
//!
//! This module has no dependencies outside `std`, which keeps it unit-testable
//! in isolation and impossible to bypass from the frontend.

use std::fmt;

/// Nesting limit for interpreters and substitutions.
const MAX_DEPTH: u32 = 6;

/// One simple command extracted from a command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Executable name: directory components stripped, `.exe` stripped, lowered.
    pub program: String,
    /// Arguments as written (no shell expansion is performed).
    pub args: Vec<String>,
    /// The segment writes to a redirection target.
    pub redirect: bool,
    /// True when the segment appears to the right of a pipe in its pipeline.
    pub piped: bool,
}

impl fmt::Display for Segment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.program)?;
        for arg in &self.args {
            write!(f, " {arg}")?;
        }
        Ok(())
    }
}

/// Result of analysing a command line.
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    /// Every simple command found, in source order.
    pub segments: Vec<Segment>,
    /// The line contains a construct we cannot resolve statically.
    pub dynamic: bool,
}

impl Analysis {
    /// True when the line contains more than one simple command.
    pub fn is_chained(&self) -> bool {
        self.segments.len() > 1
    }
}

/// Analyse `command` and return every simple command it would run.
pub fn analyze(command: &str) -> Analysis {
    let mut analysis = Analysis::default();
    collect(lex(command), 0, &mut analysis);
    analysis
}

/// Destructive constructs that are refused even when a human approves them.
/// Checked against the raw line because the tokenizer may split them apart.
pub fn is_catastrophic_raw(command: &str) -> bool {
    let compact: String = command
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    // Fork bomb, with or without spaces.
    if compact.contains(":(){:|:&};:") || compact.contains(":(){ :|:& };:") {
        return true;
    }
    // `> /dev/sda` style raw disk writes, including `dd of=...`.
    let lower = command.to_lowercase();
    for needle in ["of=/dev/sd", "of=/dev/nvme", "of=/dev/vd", ">/dev/sd", "> /dev/sd"] {
        if lower.contains(needle) {
            return true;
        }
    }
    false
}

/* ------------------------------------------------------------------ */
/* Lexer                                                              */
/* ------------------------------------------------------------------ */

#[derive(Debug, Default)]
struct RawCommand {
    words: Vec<String>,
    redirect: bool,
}

#[derive(Debug, Default)]
struct Lexed {
    commands: Vec<RawCommand>,
    /// `$(...)` and `` `...` `` payloads, already unquoted.
    substitutions: Vec<String>,
    /// Set when a construct could not be resolved (unterminated quote, ...).
    dynamic: bool,
}

fn lex(input: &str) -> Lexed {
    let chars: Vec<char> = input.chars().collect();
    let mut out = Lexed::default();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut active = false;
    let mut redirect = false;
    let mut i = 0usize;

    macro_rules! end_word {
        () => {
            if active {
                words.push(std::mem::take(&mut word));
                active = false;
            }
        };
    }
    macro_rules! end_command {
        () => {
            end_word!();
            if !words.is_empty() || redirect {
                out.commands.push(RawCommand {
                    words: std::mem::take(&mut words),
                    redirect,
                });
            }
            redirect = false;
        };
    }

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                active = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    word.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                } else {
                    out.dynamic = true;
                }
            }
            '"' => {
                active = true;
                i += 1;
                loop {
                    if i >= chars.len() {
                        out.dynamic = true;
                        break;
                    }
                    let current = chars[i];
                    if current == '"' {
                        i += 1;
                        break;
                    }
                    if current == '\\' && i + 1 < chars.len() {
                        let next = chars[i + 1];
                        if matches!(next, '"' | '\\' | '$' | '`') {
                            word.push(next);
                        } else {
                            word.push(current);
                            word.push(next);
                        }
                        i += 2;
                        continue;
                    }
                    if current == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
                        let (payload, next_index) = read_balanced(&chars, i + 2, '(', ')');
                        out.substitutions.push(payload);
                        word.push_str("$(...)");
                        i = next_index;
                        continue;
                    }
                    if current == '`' {
                        let (payload, next_index) = read_until(&chars, i + 1, '`');
                        out.substitutions.push(payload);
                        word.push_str("`...`");
                        i = next_index;
                        continue;
                    }
                    word.push(current);
                    i += 1;
                }
            }
            '\\' => {
                if i + 1 < chars.len() {
                    word.push(chars[i + 1]);
                    active = true;
                    i += 2;
                } else {
                    out.dynamic = true;
                    i += 1;
                }
            }
            '$' if i + 1 < chars.len() && chars[i + 1] == '(' => {
                let (payload, next_index) = read_balanced(&chars, i + 2, '(', ')');
                out.substitutions.push(payload);
                word.push_str("$(...)");
                active = true;
                i = next_index;
            }
            '$' if i + 1 < chars.len() && chars[i + 1] == '{' => {
                let (payload, next_index) = read_balanced(&chars, i + 2, '{', '}');
                word.push_str("${");
                word.push_str(&payload);
                word.push('}');
                active = true;
                i = next_index;
            }
            '`' => {
                let (payload, next_index) = read_until(&chars, i + 1, '`');
                out.substitutions.push(payload);
                word.push_str("`...`");
                active = true;
                i = next_index;
            }
            '#' if !active => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            ';' => {
                end_command!();
                i += 1;
            }
            '&' => {
                end_command!();
                // `&&` and `&>` are two characters, `&` alone backgrounds.
                if i + 1 < chars.len() && (chars[i + 1] == '&' || chars[i + 1] == '>') {
                    i += 1;
                    if chars[i] == '>' {
                        redirect = true;
                    }
                }
                i += 1;
            }
            '|' => {
                end_command!();
                // `||` and `|&`
                if i + 1 < chars.len() && (chars[i + 1] == '|' || chars[i + 1] == '&') {
                    i += 1;
                }
                i += 1;
            }
            '(' | ')' => {
                end_command!();
                i += 1;
            }
            '{' | '}' if !active => {
                end_command!();
                i += 1;
            }
            '>' | '<' => {
                // `2>` : the descriptor is not a command.
                if active && !word.is_empty() && word.chars().all(|ch| ch.is_ascii_digit()) {
                    word.clear();
                    active = false;
                }
                end_word!();
                redirect = true;
                i += 1;
                if i + 1 < chars.len() && chars[i] == '&' && chars[i + 1].is_ascii_digit() {
                    i += 2;
                }
            }
            other if other.is_whitespace() => {
                end_word!();
                if other == '\n' {
                    end_command!();
                }
                i += 1;
            }
            other => {
                word.push(other);
                active = true;
                i += 1;
            }
        }
    }

    end_command!();
    let _ = (active, redirect);
    out
}

/// Read a balanced pair starting *after* the opening delimiter. Nested
/// delimiters, quotes and escapes are respected.
fn read_balanced(chars: &[char], start: usize, open: char, close: char) -> (String, usize) {
    let mut depth = 1usize;
    let mut i = start;
    let mut payload = String::new();
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            payload.push(c);
            payload.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '\'' || c == '"' {
            payload.push(c);
            i += 1;
            while i < chars.len() && chars[i] != c {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    payload.push(chars[i]);
                    payload.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                payload.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                payload.push(c);
                i += 1;
            }
            continue;
        }
        if c == open {
            depth += 1;
            payload.push(c);
            i += 1;
            continue;
        }
        if c == close {
            depth -= 1;
            i += 1;
            if depth == 0 {
                break;
            }
            payload.push(c);
            continue;
        }
        payload.push(c);
        i += 1;
    }
    (payload, i)
}

fn read_until(chars: &[char], start: usize, terminator: char) -> (String, usize) {
    let mut i = start;
    let mut payload = String::new();
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            payload.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if chars[i] == terminator {
            i += 1;
            break;
        }
        payload.push(chars[i]);
        i += 1;
    }
    (payload, i)
}

/* ------------------------------------------------------------------ */
/* Segment extraction                                                 */
/* ------------------------------------------------------------------ */

fn collect(lexed: Lexed, depth: u32, analysis: &mut Analysis) {
    if depth > MAX_DEPTH {
        analysis.dynamic = true;
        return;
    }
    if lexed.dynamic {
        analysis.dynamic = true;
    }

    // `echo 'rm -rf /' | sh` : an interpreter reading its script from the pipe
    // is analysed when we see the pipeline, so remember whether a pipe was open.
    let mut piped = false;
    for command in lexed.commands {
        let words = strip_assignments(command.words);
        if !words.is_empty() {
            let segment_piped = piped;
            piped = true;
            classify_words(words, command.redirect, segment_piped, depth, analysis);
        } else if command.redirect {
            analysis.dynamic = true;
        }
    }

    for payload in lexed.substitutions {
        collect(lex(&payload), depth + 1, analysis);
    }
}

fn classify_words(
    words: Vec<String>,
    redirect: bool,
    piped: bool,
    depth: u32,
    analysis: &mut Analysis,
) {
    let mut words = strip_keywords(words);
    if words.is_empty() {
        return;
    }

    // Unwrap launchers until we reach the real program.
    loop {
        let program = normalize_program(&words[0]);
        let Some(next) = unwrap(&program, &words) else {
            // An interpreter whose script comes from stdin cannot be resolved.
            if is_interpreter(&program) && !has_inline_script(&words) && piped {
                analysis.dynamic = true;
            }
            break;
        };
        match next {
            Unwrapped::Index(index) => {
                if index >= words.len() {
                    // The wrapper had nothing to run (`timeout 5` alone).
                    return;
                }
                words = words.split_off(index);
            }
            Unwrapped::Script(script) => {
                collect(lex(&script), depth + 1, analysis);
                return;
            }
            // A non-shell interpreter runs code we cannot parse; keep the
            // segment itself and let the policy engine require approval.
            Unwrapped::Dynamic => {
                analysis.dynamic = true;
                break;
            }
        }
    }

    let program = normalize_program(&words[0]);
    let args = words[1..].to_vec();
    analysis.segments.push(Segment {
        program,
        args,
        redirect,
        piped,
    });

    // `find ... -exec rm -rf {} ;` : the payload is a command of its own.
    if words[0].eq_ignore_ascii_case("find") {
        if let Some(payload) = find_exec_payload(&words) {
            collect(lex(&payload), depth + 1, analysis);
        }
    }
}

enum Unwrapped {
    /// The wrapper was removed; continue from this word index.
    Index(usize),
    /// The wrapper carries an inline script.
    Script(String),
    /// The wrapper runs a script in a language we cannot parse.
    Dynamic,
}

fn unwrap(program: &str, words: &[String]) -> Option<Unwrapped> {
    match program {
        // Launchers that take options and then a command.
        "sudo" | "doas" => Some(Unwrapped::Index(first_non_option(
            words,
            1,
            &["-u", "-g", "-p", "-C", "-h", "-r", "-t", "-U", "-D", "-T", "-R"],
        ))),
        "env" => Some(Unwrapped::Index(skip_assignments(words))),
        "timeout" => {
            let mut index = first_non_option(words, 1, &["-s", "-k", "--signal", "--kill-after"]);
            // The first bare token is the duration.
            if index < words.len() && !words[index].starts_with('-') {
                index += 1;
            }
            Some(Unwrapped::Index(index))
        }
        "nice" => Some(Unwrapped::Index(first_non_option(
            words,
            1,
            &["-n", "--adjustment"],
        ))),
        "ionice" => Some(Unwrapped::Index(first_non_option(
            words,
            1,
            &["-c", "-n", "-p", "-P", "-u"],
        ))),
        "stdbuf" => Some(Unwrapped::Index(first_non_option(
            words,
            1,
            &["-i", "-o", "-e"],
        ))),
        "strace" | "ltrace" => Some(Unwrapped::Index(first_non_option(
            words,
            1,
            &["-o", "-e", "-p", "-s", "-u", "-E", "-a"],
        ))),
        "flock" => Some(Unwrapped::Index(first_non_option(
            words,
            1,
            &["-w", "-E", "-o", "--timeout", "--conflict-exit-code"],
        ))),
        "nohup" | "time" | "command" | "builtin" | "exec" | "setsid" | "unbuffer" | "chrt"
        | "taskset" | "numactl" => Some(Unwrapped::Index(first_non_option(words, 1, &["-p"]))),
        "watch" => {
            let index = first_non_option(words, 1, &["-n", "-d", "-q", "--interval"]);
            if index >= words.len() {
                return Some(Unwrapped::Index(index));
            }
            // `watch 'rm -rf /'` passes the command as one quoted word.
            let joined = join_words(&words[index..]);
            Some(Unwrapped::Script(joined))
        }
        "xargs" => {
            let index = first_non_option(
                words,
                1,
                &["-I", "-n", "-P", "-d", "-E", "-s", "-L", "-a"],
            );
            if index >= words.len() {
                // `... | xargs rm -rf` default echo is harmless only if no command.
                return Some(Unwrapped::Index(index));
            }
            let joined = join_words(&words[index..]);
            Some(Unwrapped::Script(joined))
        }
        "eval" => {
            let script = join_words(&words[1..]);
            Some(Unwrapped::Script(script))
        }
        "ssh" | "scp" | "sftp" => {
            // `ssh [-opts] host [command]`
            let index = first_non_option(words, 1, &["-p", "-i", "-o", "-l", "-F", "-J"]);
            let command_index = index + 1;
            if command_index >= words.len() {
                return Some(Unwrapped::Index(words.len()));
            }
            Some(Unwrapped::Script(join_words(&words[command_index..])))
        }
        "docker" | "podman" | "nerdctl" => {
            if words.get(1).map(String::as_str) == Some("exec") {
                let index = first_non_option(words, 2, &["-u", "-w", "-e", "--env", "--user"]);
                let command_index = index + 1;
                if command_index >= words.len() {
                    return Some(Unwrapped::Index(words.len()));
                }
                Some(Unwrapped::Script(join_words(&words[command_index..])))
            } else {
                None
            }
        }
        "kubectl" => {
            if words.get(1).map(String::as_str) == Some("exec") {
                let index = first_non_option(words, 2, &["-n", "-c", "--namespace", "--container"]);
                // After options come the resource and then `--` or the command.
                let mut command_index = index + 1;
                if words.get(command_index).map(String::as_str) == Some("--") {
                    command_index += 1;
                }
                if command_index >= words.len() {
                    return Some(Unwrapped::Index(words.len()));
                }
                Some(Unwrapped::Script(join_words(&words[command_index..])))
            } else {
                None
            }
        }
        "su" | "runuser" => {
            let script_flag = words
                .iter()
                .position(|word| word == "-c" || word == "--command");
            match script_flag {
                Some(index) if index + 1 < words.len() => {
                    Some(Unwrapped::Script(words[index + 1].clone()))
                }
                _ => Some(Unwrapped::Index(words.len())),
            }
        }
        "busybox" => {
            if words.len() > 1 {
                Some(Unwrapped::Index(1))
            } else {
                None
            }
        }
        // Interpreters with an inline script flag.
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "ash" | "fish" | "mksh" => {
            interpreter_payload(words).map(Unwrapped::Script)
        }
        // Interpreters whose inline script is not shell syntax: the payload is
        // reported as dynamic instead of being mis-read as a command line.
        "python" | "python2" | "python3" | "py" | "pypy" | "pypy3" => {
            interpreter_payload(words).map(|_| Unwrapped::Dynamic)
        }
        "perl" | "perl5" => {
            interpreter_payload_with(words, &["-e", "-E"]).map(|_| Unwrapped::Dynamic)
        }
        "ruby" => interpreter_payload_with(words, &["-e"]).map(|_| Unwrapped::Dynamic),
        "node" | "nodejs" | "deno" | "bun" => {
            interpreter_payload_with(words, &["-e", "--eval", "eval"]).map(|_| Unwrapped::Dynamic)
        }
        "php" => interpreter_payload_with(words, &["-r"]).map(|_| Unwrapped::Dynamic),
        "awk" | "gawk" | "mawk" | "nawk" => {
            if words.iter().any(|word| word.contains("system(") || word.contains("|")) {
                Some(Unwrapped::Dynamic)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// `sh -c "<script>"` / `sh -lc "<script>"`. Returns the script when present.
fn interpreter_payload(words: &[String]) -> Option<String> {
    for (index, word) in words.iter().enumerate().skip(1) {
        if word == "--" {
            break;
        }
        if word.starts_with("--") {
            continue;
        }
        if word.starts_with('-') && word.len() > 1 && word.contains('c') {
            return words.get(index + 1).cloned();
        }
    }
    None
}

fn interpreter_payload_with(words: &[String], flags: &[&str]) -> Option<String> {
    for (index, word) in words.iter().enumerate().skip(1) {
        if flags.contains(&word.as_str()) {
            return words.get(index + 1).cloned();
        }
        // `python3 -c"..."` is rare; `perl -ne` clusters are not scripts.
        if flags.contains(&"-c") && word.starts_with("-c") && word.len() > 2 {
            return Some(word[2..].to_string());
        }
    }
    None
}

fn has_inline_script(words: &[String]) -> bool {
    interpreter_payload(words).is_some()
}

fn is_interpreter(program: &str) -> bool {
    matches!(
        program,
        "sh" | "bash"
            | "zsh"
            | "dash"
            | "ksh"
            | "ash"
            | "fish"
            | "python"
            | "python3"
            | "perl"
            | "ruby"
            | "node"
            | "php"
            | "awk"
    )
}

/// `find <paths> -exec <command> ;` payload.
fn find_exec_payload(words: &[String]) -> Option<String> {
    let position = words
        .iter()
        .position(|word| word == "-exec" || word == "-execdir" || word == "-ok")?;
    let rest: Vec<&str> = words[position + 1..]
        .iter()
        .take_while(|word| word.as_str() != ";" && word.as_str() != "+")
        .map(String::as_str)
        .collect();
    if rest.is_empty() {
        None
    } else {
        Some(join_words_slices(&rest))
    }
}

/* ------------------------------------------------------------------ */
/* Word helpers                                                       */
/* ------------------------------------------------------------------ */

/// Rebuild a command line from already-split words, preserving the meaning of
/// words that contained quoting when they were first parsed.
fn join_words(words: &[String]) -> String {
    words
        .iter()
        .map(|word| quote_word(word))
        .collect::<Vec<_>>()
        .join(" ")
}

fn join_words_slices(words: &[&str]) -> String {
    words
        .iter()
        .map(|word| quote_word(word))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_word(word: &str) -> String {
    if word.is_empty() {
        return "''".to_string();
    }
    let needs_quotes = word.chars().any(|character| {
        character.is_whitespace()
            || matches!(
                character,
                ';' | '&' | '|' | '<' | '>' | '(' | ')' | '$' | '`' | '\'' | '"' | '#' | '{'
                    | '}' | '*' | '?' | '[' | ']' | '\\' | '!' | '~'
            )
    });
    if needs_quotes {
        format!("'{}'", word.replace('\'', r"'\''"))
    } else {
        word.to_string()
    }
}

fn strip_keywords(mut words: Vec<String>) -> Vec<String> {
    loop {
        let Some(first) = words.first() else {
            return words;
        };
        if matches!(
            first.as_str(),
            "if" | "then"
                | "else"
                | "elif"
                | "fi"
                | "while"
                | "until"
                | "do"
                | "done"
                | "for"
                | "case"
                | "esac"
                | "!"
                | "function"
        ) {
            words.remove(0);
            continue;
        }
        return words;
    }
}

fn strip_assignments(words: Vec<String>) -> Vec<String> {
    let mut index = 0;
    while index < words.len() && is_assignment(&words[index]) {
        index += 1;
    }
    words[index..].to_vec()
}

fn skip_assignments(words: &[String]) -> usize {
    let mut index = 1;
    while index < words.len() {
        let word = &words[index];
        if is_assignment(word) || word.starts_with('-') {
            // `env -u NAME` takes a value.
            if matches!(word.as_str(), "-u" | "--unset" | "-C" | "--chdir" | "-S") {
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        break;
    }
    index
}

fn is_assignment(word: &str) -> bool {
    match word.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
                && !name.starts_with(|character: char| character.is_ascii_digit())
        }
        None => false,
    }
}

fn first_non_option(words: &[String], from: usize, value_flags: &[&str]) -> usize {
    let mut index = from;
    while index < words.len() {
        let word = words[index].clone();
        if word == "--" {
            return index + 1;
        }
        if word.starts_with('-') && word.len() > 1 {
            let base = word.split('=').next().unwrap_or(word.as_str()).to_string();
            index += 1;
            if value_flags.contains(&base.as_str()) && !word.contains('=') && index < words.len() {
                index += 1;
            }
            continue;
        }
        break;
    }
    index
}

/// `/usr/bin/RM.exe` -> `rm`
pub fn normalize_program(word: &str) -> String {
    let last = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let lower = last.to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn programs(command: &str) -> Vec<String> {
        analyze(command)
            .segments
            .into_iter()
            .map(|segment| segment.program)
            .collect()
    }

    #[test]
    fn splits_command_chains() {
        assert_eq!(programs("ls -la && rm -rf ~"), vec!["ls", "rm"]);
        assert_eq!(programs("cat a; systemctl stop nginx"), vec!["cat", "systemctl"]);
        assert_eq!(programs("true || reboot"), vec!["true", "reboot"]);
        assert_eq!(programs("ps aux | grep ssh"), vec!["ps", "grep"]);
        assert_eq!(programs("echo hi\nrm -rf x"), vec!["echo", "rm"]);
        assert_eq!(programs("(cd /tmp && rm -rf x)"), vec!["cd", "rm"]);
    }

    #[test]
    fn keeps_quoted_separators_inside_words() {
        let analysis = analyze("echo 'a;b && c'");
        assert_eq!(analysis.segments.len(), 1);
        assert_eq!(analysis.segments[0].program, "echo");
        assert_eq!(analysis.segments[0].args, vec!["a;b && c"]);
    }

    #[test]
    fn follows_command_substitution() {
        assert_eq!(programs("ls $(pwd)"), vec!["ls", "pwd"]);
        assert_eq!(programs("echo `whoami`"), vec!["echo", "whoami"]);
        assert_eq!(programs("ls $(rm -rf /)"), vec!["ls", "rm"]);
    }

    #[test]
    fn normalizes_paths_and_case() {
        assert_eq!(programs("/bin/rm -rf x"), vec!["rm"]);
        assert_eq!(programs("RM.EXE -f x"), vec!["rm"]);
        assert_eq!(programs("./rm -rf x"), vec!["rm"]);
    }

    #[test]
    fn unwraps_launchers() {
        assert_eq!(programs("sudo rm -rf /"), vec!["rm"]);
        assert_eq!(programs("sudo -u www-data rm -rf x"), vec!["rm"]);
        assert_eq!(programs("env FOO=1 rm -rf x"), vec!["rm"]);
        assert_eq!(programs("timeout 30 rm -rf x"), vec!["rm"]);
        assert_eq!(programs("nohup rm -rf x &"), vec!["rm"]);
        assert_eq!(programs("nice -n 5 rm -rf x"), vec!["rm"]);
        assert_eq!(programs("find . -exec rm -rf {} ;"), vec!["find", "rm"]);
        assert_eq!(programs("xargs rm -rf"), vec!["rm"]);
    }

    #[test]
    fn unwraps_inline_scripts() {
        assert_eq!(programs("bash -c 'rm -rf /'"), vec!["rm"]);
        assert_eq!(programs("sh -lc 'rm -rf /'"), vec!["rm"]);
        assert_eq!(programs("python3 -c 'import os; os.system(\"rm -rf /\")'"), vec!["python3"]);
        assert_eq!(programs("eval rm -rf /"), vec!["rm"]);
        assert_eq!(
            programs("docker exec web sh -c 'rm -rf /'"),
            vec!["rm"]
        );
    }

    #[test]
    fn flags_unresolvable_constructs() {
        assert!(analyze("echo \"unterminated").dynamic);
        // Script fed through a pipe.
        assert!(analyze("echo 'rm -rf /' | sh").dynamic);
    }

    #[test]
    fn keeps_pipeline_position() {
        let analysis = analyze("ps aux | grep ssh");
        assert!(!analysis.segments[0].piped);
        assert!(analysis.segments[1].piped);
    }

    #[test]
    fn detects_fork_bomb_and_raw_disk_writes() {
        assert!(is_catastrophic_raw(":(){ :|:& };:"));
        assert!(is_catastrophic_raw("dd if=/dev/zero of=/dev/sda bs=1M"));
        assert!(!is_catastrophic_raw("echo of=/dev/null"));
    }

    #[test]
    fn tolerates_redirections() {
        let analysis = analyze("ls -la > out.txt");
        assert_eq!(analysis.segments.len(), 1);
        assert!(analysis.segments[0].redirect);
        assert_eq!(analysis.segments[0].program, "ls");
        // File descriptors are not commands.
        assert_eq!(programs("cmd 2>&1 | tee log"), vec!["cmd", "tee"]);
    }
}
