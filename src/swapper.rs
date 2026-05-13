use clap::{Command, Arg, ArgAction};
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

trait Executor {
  fn execute(&mut self, args: Vec<String>) -> String;
  fn last_executed(&self) -> Option<Vec<String>>;
}

struct RealShell {
  executed: Option<Vec<String>>,
}

impl RealShell {
  fn new() -> RealShell {
    RealShell { executed: None }
  }
}

impl Executor for RealShell {
  fn execute(&mut self, args: Vec<String>) -> String {
    let execution = std::process::Command::new(args[0].as_str())
      .args(&args[1..])
      .output()
      .expect("Couldn't run it");

    self.executed = Some(args);

    let output: String = String::from_utf8_lossy(&execution.stdout).into();

    output.trim_end().to_string()
  }

  fn last_executed(&self) -> Option<Vec<String>> {
    self.executed.clone()
  }
}

const TMP_FILE: &str = "/tmp/thumbs-last";

#[allow(dead_code)]
fn dbg(msg: &str) {
  let mut file = std::fs::OpenOptions::new()
    .create(true)
    .write(true)
    .append(true)
    .open("/tmp/thumbs.log")
    .expect("Unable to open log file");

  writeln!(&mut file, "{}", msg).expect("Unable to write log file");
}

fn parse_option_line(line: &str) -> Option<(String, String)> {
  if !line.starts_with("@thumbs-") {
    return None;
  }
  let line = line.trim();
  if let Some(space_idx) = line.find(|c: char| c.is_whitespace()) {
    let name = &line["@thumbs-".len()..space_idx];
    let mut value = line[space_idx..].trim().to_string();

    if (value.starts_with('"') && value.ends_with('"')) || (value.starts_with('\'') && value.ends_with('\'')) {
      value.remove(0);
      value.pop();
      value = value.replace("\\\"", "\"").replace("\\\\", "\\").replace("\\~", "~");
    }
    return Some((name.to_string(), value));
  }
  None
}

pub struct Swapper<'a> {
  executor: Box<&'a mut dyn Executor>,
  dir: String,
  command: String,
  upcase_command: String,
  multi_command: String,
  osc52: bool,
  active_pane_id: Option<String>,
  active_pane_height: Option<i32>,
  active_pane_scroll_position: Option<i32>,
  active_pane_zoomed: Option<bool>,
  thumbs_pane_id: Option<String>,
  content: Option<String>,
  signal: String,
}

impl<'a> Swapper<'a> {
  fn new(
    executor: Box<&'a mut dyn Executor>,
    dir: String,
    command: String,
    upcase_command: String,
    multi_command: String,
    osc52: bool,
  ) -> Swapper {
    let since_the_epoch = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("Time went backwards");
    let signal = format!("thumbs-finished-{}", since_the_epoch.as_secs());

    Swapper {
      executor,
      dir,
      command,
      upcase_command,
      multi_command,
      osc52,
      active_pane_id: None,
      active_pane_height: None,
      active_pane_scroll_position: None,
      active_pane_zoomed: None,
      thumbs_pane_id: None,
      content: None,
      signal,
    }
  }

  fn read_options(&mut self) -> String {
    #[cfg(not(test))]
    {
      let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| {
          let output = std::process::Command::new("id")
            .args(&["-un"])
            .output();
          match output {
            Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
            Err(_) => "".to_string(),
          }
        });

      let file_path = format!("/tmp/thumbs-options-{}.txt", user);

      if !user.is_empty() {
        if let Ok(content) = std::fs::read_to_string(&file_path) {
          return content;
        }
      }
    }

    // Fallback to tmux show -g (always used in tests for hermeticity)
    let options_command = vec!["tmux", "show", "-g"];
    let params: Vec<String> = options_command.iter().map(|arg| arg.to_string()).collect();
    self.executor.execute(params)
  }

  pub fn capture_active_pane(&mut self) {
    let active_command = vec![
      "tmux",
      "display-message",
      "-p",
      "#{pane_id}:#{?pane_in_mode,1,0}:#{pane_height}:#{scroll_position}:#{window_zoomed_flag}:active",
    ];

    let output = self
      .executor
      .execute(active_command.iter().map(|arg| arg.to_string()).collect());

    let chunks: Vec<&str> = output.split(':').collect();

    let pane_id = chunks.get(0).unwrap();

    self.active_pane_id = Some(pane_id.to_string());

    let pane_height = chunks
      .get(2)
      .unwrap()
      .parse()
      .expect("Unable to retrieve pane height");

    self.active_pane_height = Some(pane_height);

    if chunks.get(1).unwrap().to_string() == "1" {
      let pane_scroll_position = chunks
        .get(3)
        .unwrap()
        .parse()
        .expect("Unable to retrieve pane scroll");

      self.active_pane_scroll_position = Some(pane_scroll_position);
    }

    let zoomed_pane = *chunks.get(4).expect("Unable to retrieve zoom pane property") == "1";

    self.active_pane_zoomed = Some(zoomed_pane);
  }

  pub fn execute_thumbs(&mut self) {
    let options = self.read_options();
    let lines: Vec<&str> = options.split('\n').collect();

    let mut args = Vec::new();

    for line in lines {
      if let Some((name, value)) = parse_option_line(line) {
        match name.as_str() {
          "command" => self.command = value,
          "upcase-command" => self.upcase_command = value,
          "multi-command" => self.multi_command = value,
          "osc52" => self.osc52 = value == "1" || value == "true",
          "reverse" | "unique" | "contrast" | "faint" => {
            if value == "1" || value == "true" {
              args.push(format!("--{}", name));
            }
          }
          "alphabet" | "position" | "fg-color" | "bg-color" | "alt-bg-color" | "dim-color" |
          "hint-bg-color" | "hint-fg-color" | "select-fg-color" | "select-bg-color" |
          "multi-fg-color" | "multi-bg-color" => {
            args.push(format!("--{}", name));
            args.push(format!("'{}'", value));
          }
          _ if name.starts_with("regexp") => {
            args.push("--regexp".to_string());
            args.push(format!("'{}'", value.replace("\\\\", "\\")));
          }
          _ => {}
        }
      }
    }

    let active_pane_id = self.active_pane_id.as_mut().unwrap().clone();
    let height = self.active_pane_height.unwrap_or(i32::MAX);

    // 1. Capture pane content synchronously before swap
    let mut capture_args = vec![
      "capture-pane".to_string(),
      "-t".to_string(),
      active_pane_id.to_string(),
      "-p".to_string(),
      "-e".to_string(),
    ];
    
    if let (Some(pane_height), Some(scroll_position)) = (self.active_pane_height, self.active_pane_scroll_position) {
      capture_args.push("-S".to_string());
      capture_args.push(format!("{}", -scroll_position));
      capture_args.push("-E".to_string());
      capture_args.push(format!("{}", pane_height - scroll_position - 1));
    }

    let params: Vec<String> = [vec!["tmux".to_string()], capture_args].concat();
    let captured_text = self.executor.execute(params);

    // 2. Trim trailing spaces and empty lines, and tail to height in Rust
    let mut captured_lines: Vec<&str> = captured_text
      .split('\n')
      .map(|line| line.trim_end_matches(|c: char| c == ' ' || c == '\t'))
      .collect();

    while let Some(last_line) = captured_lines.last() {
      if last_line.is_empty() {
        captured_lines.pop();
      } else {
        break;
      }
    }

    let tail_len = std::cmp::min(height as usize, captured_lines.len());
    let visible_lines = if captured_lines.is_empty() {
      &[]
    } else {
      &captured_lines[captured_lines.len() - tail_len..]
    };
    let final_text = visible_lines.join("\n");

    std::fs::write("/tmp/thumbs-captured.log", final_text).unwrap();

    // 3. Construct pane command that just reads from the pre-captured log
    let active_pane_zoomed = self.active_pane_zoomed.as_mut().unwrap().clone();
    let zoom_command = if active_pane_zoomed {
      format!("tmux resize-pane -t {} -Z;", active_pane_id)
    } else {
      "".to_string()
    };

    let pane_command = format!(
        "({dir}/target/release/thumbs -f '%U:%H' -t {tmp} --input /tmp/thumbs-captured.log {args}) 2>/tmp/thumbs-stderr.log; tmux swap-pane -t {active_pane_id}; {zoom_command} tmux wait-for -S {signal}",
        active_pane_id = active_pane_id,
        dir = self.dir,
        tmp = TMP_FILE,
        args = args.join(" "),
        zoom_command = zoom_command,
        signal = self.signal
    );

    let thumbs_command = vec![
      "tmux",
      "new-window",
      "-P",
      "-F",
      "#{pane_id}",
      "-d",
      "-n",
      "[thumbs]",
      pane_command.as_str(),
    ];

    let params: Vec<String> = thumbs_command.iter().map(|arg| arg.to_string()).collect();

    self.thumbs_pane_id = Some(self.executor.execute(params));
  }

  pub fn swap_panes(&mut self) {
    let active_pane_id = self.active_pane_id.as_mut().unwrap().clone();
    let thumbs_pane_id = self.thumbs_pane_id.as_mut().unwrap().clone();

    let swap_command = vec![
      "tmux",
      "swap-pane",
      "-d",
      "-s",
      active_pane_id.as_str(),
      "-t",
      thumbs_pane_id.as_str(),
    ];

    let params = swap_command
      .iter()
      .filter(|&s| !s.is_empty())
      .map(|arg| arg.to_string())
      .collect();

    self.executor.execute(params);
  }

  pub fn resize_pane(&mut self) {
    let active_pane_zoomed = self.active_pane_zoomed.as_mut().unwrap().clone();

    if !active_pane_zoomed {
      return;
    }

    let thumbs_pane_id = self.thumbs_pane_id.as_mut().unwrap().clone();

    let resize_command = vec!["tmux", "resize-pane", "-t", thumbs_pane_id.as_str(), "-Z"];

    let params = resize_command
      .iter()
      .filter(|&s| !s.is_empty())
      .map(|arg| arg.to_string())
      .collect();

    self.executor.execute(params);
  }

  pub fn wait_thumbs(&mut self) {
    let wait_command = vec!["tmux", "wait-for", self.signal.as_str()];
    let params = wait_command.iter().map(|arg| arg.to_string()).collect();

    self.executor.execute(params);
  }

  pub fn retrieve_content(&mut self) {
    let retrieve_command = vec!["cat", TMP_FILE];
    let params = retrieve_command.iter().map(|arg| arg.to_string()).collect();

    self.content = Some(self.executor.execute(params));
  }

  pub fn destroy_content(&mut self) {
    let retrieve_command = vec!["rm", TMP_FILE];
    let params = retrieve_command.iter().map(|arg| arg.to_string()).collect();

    self.executor.execute(params);
  }

  pub fn send_osc52(&mut self) {}

  pub fn execute_command(&mut self) {
    let content = self.content.clone().unwrap();
    let items: Vec<&str> = content.split('\n').collect();

    if items.len() > 1 {
      let text = items
        .iter()
        .map(|item| item.splitn(2, ':').last().unwrap())
        .collect::<Vec<&str>>()
        .join(" ");

      self.execute_final_command(&text, &self.multi_command.clone());

      return;
    }

    // Only one item
    let item: &str = items.first().unwrap();

    let mut splitter = item.splitn(2, ':');

    if let Some(upcase) = splitter.next() {
      if let Some(text) = splitter.next() {
        if self.osc52 {
          use base64::Engine;
          let base64_text = base64::prelude::BASE64_STANDARD.encode(text.as_bytes());
          let osc_seq = format!("\x1b]52;0;{}\x07", base64_text);
          let tmux_seq = format!("\x1bPtmux;{}\x1b\\", osc_seq.replace("\x1b", "\x1b\x1b"));

          // FIXME: Review if this comment is still rellevant
          //
          // When the user selects a match:
          // 1. The `rustbox` object created in the `viewbox` above is dropped.
          // 2. During its `drop`, the `rustbox` object sends a CSI 1049 escape
          //    sequence to tmux.
          // 3. This escape sequence causes the `window_pane_alternate_off` function
          //    in tmux to be called.
          // 4. In `window_pane_alternate_off`, tmux sets the needs-redraw flag in the
          //    pane.
          // 5. If we print the OSC copy escape sequence before the redraw is completed,
          //    tmux will *not* send the sequence to the host terminal. See the following
          //    call chain in tmux: `input_dcs_dispatch` -> `screen_write_rawstring`
          //    -> `tty_write` -> `tty_client_ready`. In this case, `tty_client_ready`
          //    will return false, thus preventing the escape sequence from being sent.
          //
          // Therefore, for now we wait a little bit here for the redraw to finish.
          std::thread::sleep(std::time::Duration::from_millis(100));

          std::io::stdout().write_all(tmux_seq.as_bytes()).unwrap();
          std::io::stdout().flush().unwrap();
        }

        let execute_command = if upcase.trim_end() == "true" {
          self.upcase_command.clone()
        } else {
          self.command.clone()
        };

        // The command we run has two arguments:
        //  * The first arg is the (trimmed) text. This gets stored in a variable, in order to
        //    preserve quoting and special characters.
        //
        //  * The second argument is the user's command, with the '{}' token replaced with an
        //    unquoted reference to the variable containing the text.
        //
        // The reference is unquoted, unfortunately, because the token may already have been
        // spliced into a string (e.g 'tmux display-message "Copied {}"'), and it's impossible (or
        // at least exceedingly difficult) to determine the correct quoting level.
        //
        // The alternative of literally splicing the text into the command is bad and it causes all
        // kinds of harmful escaping issues that the user cannot reasonable avoid.
        //
        // For example, imagine some pattern matched the text "foo;rm *" and the user's command was
        // an innocuous "echo {}". With literal splicing, we would run the command "echo foo;rm *".
        // That's BAD. Without splicing, instead we execute "echo ${THUMB}" which does mostly the
        // right thing regardless the contents of the text. (At worst, bash will word-separate the
        // unquoted variable; but it won't _execute_ those words in common scenarios).
        //
        // Ideally user commands would just use "${THUMB}" to begin with rather than having any
        // sort of ad-hoc string splicing here at all, and then they could specify the quoting they
        // want, but that would break backwards compatibility.
        self.execute_final_command(text.trim_end(), &execute_command);
      }
    }
  }

  pub fn execute_final_command(&mut self, text: &str, execute_command: &str) {
    let mut final_command = str::replace(execute_command, "{}", "${THUMB}");
    if let Some(ref active_pane) = self.active_pane_id {
      final_command = str::replace(&final_command, "{active_pane}", active_pane);
    }
    let retrieve_command = vec![
      "bash",
      "-c",
      "THUMB=\"$1\"; eval \"$2\"",
      "--",
      text,
      final_command.as_str(),
    ];

    let params = retrieve_command.iter().map(|arg| arg.to_string()).collect();

    self.executor.execute(params);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  struct TestShell {
    outputs: Vec<String>,
    executed: Option<Vec<String>>,
  }

  impl TestShell {
    fn new(outputs: Vec<String>) -> TestShell {
      TestShell {
        executed: None,
        outputs,
      }
    }
  }

  impl Executor for TestShell {
    fn execute(&mut self, args: Vec<String>) -> String {
      self.executed = Some(args);
      self.outputs.pop().unwrap()
    }

    fn last_executed(&self) -> Option<Vec<String>> {
      self.executed.clone()
    }
  }

  #[test]
  fn retrieve_active_pane() {
    let last_command_outputs = vec!["%97:0:24::0:active".to_string()];
    let mut executor = TestShell::new(last_command_outputs);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );

    swapper.capture_active_pane();

    assert_eq!(swapper.active_pane_id.unwrap(), "%97");
  }

  #[test]
  fn swap_panes() {
    let last_command_outputs = vec![
      "".to_string(),
      "%100".to_string(),
      "/tmp/some_path\n".to_string(),
      "".to_string(),
      "%98:0:24::0:active".to_string(),
    ];
    let mut executor = TestShell::new(last_command_outputs);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );

    swapper.capture_active_pane();
    swapper.execute_thumbs();
    swapper.swap_panes();

    let expectation = vec!["tmux", "swap-pane", "-d", "-s", "%98", "-t", "%100"];

    assert_eq!(executor.last_executed().unwrap(), expectation);
  }

  #[test]
  fn quoted_execution() {
    let last_command_outputs = vec!["Blah blah blah, the ignored user script output".to_string()];
    let mut executor = TestShell::new(last_command_outputs);

    let user_command = "echo \"{active_pane} {}\"".to_string();
    let upcase_command = "open \"{}\"".to_string();
    let multi_command = "open \"{}\"".to_string();
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "".to_string(),
      user_command,
      upcase_command,
      multi_command,
      false,
    );

    swapper.active_pane_id = Some("%99".to_string());
    swapper.content = Some(format!(
      "{do_upcase}:{thumb_text}",
      do_upcase = false,
      thumb_text = "foobar;rm *",
    ));
    swapper.execute_command();

    let expectation = vec![
      "bash",
      // The actual shell command:
      "-c",
      "THUMB=\"$1\"; eval \"$2\"",
      // $0: The non-existent program name.
      "--",
      // $1: The value assigned to THUMB above.
      //     Not interpreted as a shell expression!
      "foobar;rm *",
      // $2: The user script, with {} replaced with ${THUMB},
      //     and will be eval'd with THUMB in scope.
      "echo \"%99 ${THUMB}\"",
    ];

    assert_eq!(executor.last_executed().unwrap(), expectation);
  }

  #[test]
  fn test_parse_option_line() {
    assert_eq!(
      parse_option_line(r#"@thumbs-command "tmux set-buffer -w {}""#),
      Some(("command".to_string(), "tmux set-buffer -w {}".to_string()))
    );
    assert_eq!(
      parse_option_line(r#"@thumbs-upcase-command "\~/.dotfiles/tmux/run-tmux-fingers \"{}\"""#),
      Some(("upcase-command".to_string(), r#"~/.dotfiles/tmux/run-tmux-fingers "{}""#.to_string()))
    );
    assert_eq!(
      parse_option_line(r#"@thumbs-unique 1"#),
      Some(("unique".to_string(), "1".to_string()))
    );
    assert_eq!(
      parse_option_line(r#"@thumbs-regexp-1 "[0-9]+""#),
      Some(("regexp-1".to_string(), "[0-9]+".to_string()))
    );
    assert_eq!(
      parse_option_line(r#"not-a-thumbs-option value"#),
      None
    );
  }
}

fn app_args() -> clap::ArgMatches {
  Command::new("tmux-thumbs")
    .version(env!("CARGO_PKG_VERSION"))
    .about("A lightning fast version of tmux-fingers, copy/pasting tmux like vimium/vimperator")
    .arg(
      Arg::new("dir")
        .help("Directory where to execute thumbs")
        .long("dir")
        .num_args(1)
        .default_value(""),
    )
    .arg(
      Arg::new("command")
        .help("Command to execute after choose a hint")
        .long("command")
        .num_args(1)
        .default_value("tmux set-buffer -- \"{}\" && tmux display-message \"Copied {}\""),
    )
    .arg(
      Arg::new("upcase_command")
        .help("Command to execute after choose a hint, in upcase")
        .long("upcase-command")
        .num_args(1)
        .default_value("tmux set-buffer -- \"{}\" && tmux paste-buffer && tmux display-message \"Copied {}\""),
    )
    .arg(
      Arg::new("multi_command")
        .help("Command to execute after choose multiple hints")
        .long("multi-command")
        .num_args(1)
        .default_value("tmux set-buffer -- \"{}\" && tmux paste-buffer && tmux display-message \"Multi copied {}\""),
    )
    .arg(
      Arg::new("osc52")
        .help("Print OSC52 copy escape sequence in addition to running the pick command")
        .long("osc52")
        .short('o')
        .action(ArgAction::SetTrue),
    )
    .get_matches()
}

fn main() -> std::io::Result<()> {
  let args = app_args();
  let dir = args.get_one::<String>("dir").unwrap();
  let command = args.get_one::<String>("command").unwrap();
  let upcase_command = args.get_one::<String>("upcase_command").unwrap();
  let multi_command = args.get_one::<String>("multi_command").unwrap();
  let osc52 = args.get_flag("osc52");

  if dir.is_empty() {
    panic!("Invalid tmux-thumbs execution. Are you trying to execute tmux-thumbs directly?")
  }

  let mut executor = RealShell::new();
  let mut swapper = Swapper::new(
    Box::new(&mut executor),
    dir.to_string(),
    command.to_string(),
    upcase_command.to_string(),
    multi_command.to_string(),
    osc52,
  );

  swapper.capture_active_pane();
  swapper.execute_thumbs();
  swapper.swap_panes();
  swapper.resize_pane();
  swapper.wait_thumbs();
  swapper.retrieve_content();
  swapper.destroy_content();
  swapper.execute_command();

  Ok(())
}
