//! Standard input through the real binary: pipelines into a script, a package target's grant,
//! and an application that keeps its frames while stdin says nothing. Every child gets a pipe,
//! never this process's stdin, and a deadline.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::common::{Path, PathBuf, fs, jess, temp_dir_path};

const DEADLINE: Duration = Duration::from_secs(60);

/// A scratch folder for one test, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let root = temp_dir_path(name);
        drop(fs::remove_dir_all(&root));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

fn piped(mut command: Command) -> Child {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// `wait_with_output`, with a deadline: the output goes to a thread, and the child dies at the
/// deadline, which ends the thread.
fn finish(child: Child) -> Output {
    let id = child.id();
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || send.send(child.wait_with_output()));
    if let Ok(output) = receive.recv_timeout(DEADLINE) {
        return output.unwrap();
    }
    kill(id);
    panic!("jess outlived its deadline");
}

#[cfg(unix)]
fn kill(id: u32) {
    drop(Command::new("kill").arg("-9").arg(id.to_string()).status());
}

#[cfg(windows)]
fn kill(id: u32) {
    drop(Command::new("taskkill").args(["/F", "/PID", &id.to_string()]).status());
}

/// Runs a script reading `input`, which ends with the pipe closed.
fn pipeline(script: &str, input: &[u8]) -> Output {
    let scratch = Scratch::new(&format!("stdin-{:x}", {
        let mut hash = 0u64;
        for byte in script.bytes().chain(input.iter().copied()) {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
        hash
    }));
    let path = scratch.0.join("script.jsl");
    fs::write(&path, script).unwrap();
    let mut command = jess();
    command.arg("run").arg(&path);
    let mut child = piped(command);
    let mut pipe = child.stdin.take().unwrap();
    // A write can fail if the script stopped early; the assertions on its output say so better.
    drop(pipe.write_all(input));
    drop(pipe);
    finish(child)
}

/// Prints each line as a Luau-escaped `%q`, then `EOF`.
const LINES: &str = r"
local process = require '@dream/process'
local input = assert(process.openStdin())

while true do
  local line, message, kind = input:readLine(64)
  if kind == 'wouldBlock' then
    input:wait(1000)
  elseif message then
    error(message)
  elseif line == nil then
    break
  else
    print(string.format('%q', line))
  end
end

print('EOF')
input:close()
";

fn lines_of(input: &[u8]) -> String {
    let output = pipeline(LINES, input);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

#[test]
fn a_pipeline_arrives_line_by_line_and_eof_is_not_an_empty_line() {
    assert_eq!(lines_of(b"one\n"), "\"one\"\nEOF\n");
    assert_eq!(lines_of(b"one\ntwo\n"), "\"one\"\n\"two\"\nEOF\n");
    assert_eq!(lines_of(b"\n\nthree\n"), "\"\"\n\"\"\n\"three\"\nEOF\n");
    assert_eq!(lines_of(b"unterminated"), "\"unterminated\"\nEOF\n");
    assert_eq!(lines_of(b"crlf\r\nlone\rcr\n"), "\"crlf\"\n\"lone\\rcr\"\nEOF\n");
    assert_eq!(lines_of(b""), "EOF\n");
}

#[test]
fn binary_input_comes_through_byte_for_byte() {
    let everything: Vec<u8> = (0..=255).cycle().take(100_000).collect();
    let output = pipeline(
        r"
local process = require '@dream/process'
local input = assert(process.openStdin())
local pieces = {}

while true do
  local bytes, message, kind = input:read(4096)
  if kind == 'wouldBlock' then
    input:wait(1000)
  elseif message then
    error(message)
  elseif bytes == nil then
    break
  else
    table.insert(pieces, bytes)
  end
end

process.write('stdout', table.concat(pieces))
",
        &everything,
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stdout == everything, "{} bytes came back", output.stdout.len());
}

#[test]
fn a_line_past_its_bound_is_refused_and_the_next_call_carries_on_with_the_rest() {
    let output = pipeline(
        r"
local process = require '@dream/process'
local input = assert(process.openStdin())

while true do
  local line, message, kind = input:readLine(4)
  if kind == 'wouldBlock' then
    input:wait(1000)
  elseif kind then
    print(kind)
  elseif message then
    error(message)
  elseif line == nil then
    break
  else
    print(line)
  end
end
",
        b"toolongline\nok\n",
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    assert_eq!(stdout, "invalidInput\ninvalidInput\ne\nok\n");
}

/// A package whose command target declares `capabilities`, reading stdin.
fn tool(root: &Path, capabilities: &str, entry: &str) -> PathBuf {
    let tool = root.join("tool");
    fs::create_dir_all(tool.join("src")).unwrap();
    fs::write(
        tool.join("jess.toml"),
        format!(
            "manifest_version = 1\n\n[package]\nid = \"org.example.reader\"\nversion = \"1.0.0\"\njess_api = \"1\"\ndefault_target = \"tool\"\ninclude = [\"src\"]\n\n[targets.tool]\nkind = \"command\"\nentry = \"src/main.jsl\"\ncapabilities = {capabilities}\n"
        ),
    )
    .unwrap();
    fs::write(tool.join("src/main.jsl"), entry).unwrap();
    tool
}

#[test]
fn a_target_reads_stdin_only_if_it_declares_the_terminal() {
    const ENTRY: &str = r"
local process = require '@dream/process'
print(jess.granted('process.stdio'))
local input = assert(process.openStdin())
while true do
  local line, _, kind = input:readLine()
  if kind == 'wouldBlock' then
    input:wait(1000)
  else
    print(line)
    return 0
  end
end
";
    let scratch = Scratch::new("stdin-target");
    let allowed = tool(&scratch.0, "[\"process.stdio\"]", ENTRY);
    let mut command = jess();
    command.args(["package", "run"]).arg(&allowed);
    let mut child = piped(command);
    child.stdin.take().unwrap().write_all(b"typed\n").unwrap();
    let output = finish(child);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"), "true\ntyped\n");

    let scratch = Scratch::new("stdin-target-denied");
    let denied = tool(&scratch.0, "[]", ENTRY);
    let mut command = jess();
    command.args(["package", "run"]).arg(&denied);
    let mut child = piped(command);
    let pipe = child.stdin.take();
    let output = finish(child);
    drop(pipe);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("needs the 'process.stdio' capability"), "{stderr}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"), "false\n");
}

/// An application reading stdin from a repeating timer, printing `frame` each frame it runs.
fn application(root: &Path) -> PathBuf {
    let app = root.join("app");
    fs::create_dir_all(app.join("src")).unwrap();
    fs::write(
        app.join("jess.toml"),
        "manifest_version = 1\n\n[package]\nid = \"org.example.listener\"\nversion = \"1.0.0\"\njess_api = \"1\"\ndefault_target = \"app\"\ninclude = [\"src\"]\n\n[managed.main]\nmodule = \"src/main.jsl\"\n\n[targets.app]\nkind = \"application\"\nentry = \"src/entry.jsl\"\ncapabilities = [\"process.stdio\"]\n",
    )
    .unwrap();
    fs::write(app.join("src/entry.jsl"), "return 0\n").unwrap();
    fs::write(
        app.join("src/main.jsl"),
        r"
local application = require '@jess/application'
local process = require '@dream/process'
local timer = require '@jess/timer'
local input

return {
  on = {
    host = {
      startup = function()
        input = assert(process.openStdin())
        local waited, refused = pcall(input.wait, input, 0)
        print(`wait {waited} {string.find(refused, 'would block', 1, true) ~= nil}`)
        timer.every(1, function()
          local line, message, kind = input:readLine()
          if kind == 'wouldBlock' then return end
          if message then error(message) end
          print(`read {line}`)
          application.exit(line == 'stop' and 7 or 8)
        end)
      end,
      update = function()
        print('frame')
      end,
    },
  },
}
",
    )
    .unwrap();
    app
}

/// Reads stdout lines until `done` says enough, with the deadline counted from the start.
fn read_until(lines: &mpsc::Receiver<String>, mut done: impl FnMut(&str) -> bool) {
    let started = Instant::now();
    loop {
        let line = lines
            .recv_timeout(DEADLINE.saturating_sub(started.elapsed()))
            .expect("jess stopped talking");
        if done(&line) {
            return;
        }
    }
}

#[test]
fn an_application_keeps_its_frames_while_stdin_says_nothing_and_hears_it_when_it_speaks() {
    let scratch = Scratch::new("stdin-application");
    let app = application(&scratch.0);
    let mut command = jess();
    command.args(["package", "run"]).arg(&app);
    let mut child = piped(command);
    let mut pipe = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (send, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if send.send(line).is_err() {
                return;
            }
        }
    });

    read_until(&lines, |line| {
        assert_ne!(line, "wait true true");
        line.starts_with("wait false true")
    });
    let mut frames = 0;
    read_until(&lines, |line| {
        frames += u32::from(line == "frame");
        frames == 10
    });
    pipe.write_all(b"stop\n").unwrap();
    pipe.flush().unwrap();
    read_until(&lines, |line| line == "read stop");
    let id = child.id();
    let (done, status) = mpsc::channel();
    std::thread::spawn(move || done.send(child.wait()));
    let Ok(code) = status.recv_timeout(DEADLINE) else {
        kill(id);
        panic!("the application did not stop");
    };
    drop(pipe);
    assert_eq!(code.unwrap().code(), Some(7));
}

#[test]
fn an_application_stops_with_a_read_still_blocked() {
    let scratch = Scratch::new("stdin-blocked");
    let app = application(&scratch.0);
    let mut command = jess();
    command.args(["package", "run"]).arg(&app).args(["--frames", "5"]);
    let mut child = piped(command);
    // Held open past the child's end: the reader thread is in a read nothing will finish.
    let pipe = child.stdin.take();
    let output = finish(child);
    drop(pipe);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("frame"));
}

#[test]
fn stdin_closed_by_the_parent_at_once_is_eof_and_null_stdin_too() {
    let scratch = Scratch::new("stdin-null");
    let path = scratch.0.join("script.jsl");
    fs::write(&path, LINES).unwrap();
    let output = jess()
        .arg("run")
        .arg(&path)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"), "EOF\n");
}

#[test]
fn a_standalone_script_is_told_timers_need_frames_and_where_to_go_instead() {
    let scratch = Scratch::new("timer-standalone");
    let path = scratch.0.join("script.jsl");
    fs::write(&path, "require('@jess/timer').after(1, function() print('never') end)\n").unwrap();
    let output = jess().arg("run").arg(&path).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("needs a running application") && stderr.contains("@jess/net"),
        "{stderr}"
    );
}

#[test]
fn an_application_s_timer_runs_on_real_frames_in_its_module_s_context() {
    let scratch = Scratch::new("timer-application");
    let app = scratch.0.join("app");
    fs::create_dir_all(app.join("src")).unwrap();
    fs::write(
        app.join("jess.toml"),
        "manifest_version = 1\n\n[package]\nid = \"org.example.ticker\"\nversion = \"1.0.0\"\njess_api = \"1\"\ndefault_target = \"app\"\ninclude = [\"src\"]\n\n[managed.main]\nmodule = \"src/main.jsl\"\n\n[targets.app]\nkind = \"application\"\nentry = \"src/entry.jsl\"\n",
    )
    .unwrap();
    fs::write(app.join("src/entry.jsl"), "return 0\n").unwrap();
    fs::write(
        app.join("src/main.jsl"),
        r"
local application = require '@jess/application'
local timer = require '@jess/timer'

return {
  on = {
    host = {
      startup = function()
        timer.after(30, function()
          error('this timer broke')
        end)
        timer.after(60, function()
          print(`fired at or after 60: {timer.now() >= 60}`)
          application.exit(5)
        end)
      end,
    },
  },
}
",
    )
    .unwrap();
    let output = jess().args(["package", "run"]).arg(&app).output().unwrap();
    assert_eq!(output.status.code(), Some(5), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
        "fired at or after 60: true\n"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("org.example.ticker/main during timer") && stderr.contains("this timer broke"),
        "{stderr}"
    );
}
