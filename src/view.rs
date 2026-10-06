use super::*;
use std::char;
use std::io::{stdout, Read, Write};
use termion::event::Key;
use termion::input::TermRead;
use termion::raw::IntoRawMode;
use termion::screen::IntoAlternateScreen;
use termion::{color, cursor};

use unicode_width::UnicodeWidthStr;

pub struct View<'a> {
  state: &'a state::State<'a>,
  skip: usize,
  multi: bool,
  contrast: bool,
  faint: bool,
  position: &'a str,
  matches: Vec<state::Match<'a>>,
  select_foreground_color: Box<dyn color::Color>,
  select_background_color: Box<dyn color::Color>,
  multi_foreground_color: Box<dyn color::Color>,
  multi_background_color: Box<dyn color::Color>,
  foreground_color: Box<dyn color::Color>,
  background_color: Box<dyn color::Color>,
  alt_background_color: Option<Box<dyn color::Color>>,
  dim_color: Option<Box<dyn color::Color>>,
  hint_background_color: Box<dyn color::Color>,
  hint_foreground_color: Box<dyn color::Color>,
  chosen: Vec<(String, bool)>,
}

enum CaptureEvent {
  Exit,
  Hint,
}

impl<'a> View<'a> {
  pub fn new(
    state: &'a state::State<'a>,
    multi: bool,
    reverse: bool,
    unique: bool,
    contrast: bool,
    faint: bool,
    position: &'a str,
    select_foreground_color: Box<dyn color::Color>,
    select_background_color: Box<dyn color::Color>,
    multi_foreground_color: Box<dyn color::Color>,
    multi_background_color: Box<dyn color::Color>,
    foreground_color: Box<dyn color::Color>,
    background_color: Box<dyn color::Color>,
    alt_background_color: Option<Box<dyn color::Color>>,
    dim_color: Option<Box<dyn color::Color>>,
    hint_foreground_color: Box<dyn color::Color>,
    hint_background_color: Box<dyn color::Color>,
  ) -> View<'a> {
    let matches = state.matches(reverse, unique);
    let skip = if reverse { matches.len().saturating_sub(1) } else { 0 };

    View {
      state,
      skip,
      multi,
      contrast,
      faint,
      position,
      matches,
      select_foreground_color,
      select_background_color,
      multi_foreground_color,
      multi_background_color,
      foreground_color,
      background_color,
      alt_background_color,
      dim_color,
      hint_foreground_color,
      hint_background_color,
      chosen: vec![],
    }
  }

  pub fn prev(&mut self) {
    if self.skip > 0 {
      self.skip -= 1;
    }
  }

  pub fn next(&mut self) {
    if self.skip < self.matches.len().saturating_sub(1) {
      self.skip += 1;
    }
  }

  fn make_hint_text(&self, hint: &str) -> String {
    if self.contrast {
      format!("[{}]", hint)
    } else {
      hint.to_string()
    }
  }

  fn toggle_or_choose(&mut self, text: &str, upcase: bool) {
    if self.multi {
      if let Some(pos) = self.chosen.iter().position(|(t, _)| t == text) {
        self.chosen.remove(pos);
      } else {
        self.chosen.push((text.to_string(), upcase));
      }
    } else {
      self.chosen.push((text.to_string(), upcase));
    }
  }

  fn render(&self, stdout: &mut dyn Write, typed_hint: &str) -> () {
    let mut buf: Vec<u8> = Vec::with_capacity(16384);
    write!(buf, "{}", cursor::Hide).unwrap();

    let (width, height) = termion::terminal_size().unwrap_or((80, 24));
    let w = if width == 0 { 80 } else { width as usize };
    let h = if height == 0 { 24 } else { height as usize };

    // Render background lines
    for (index, line) in self.state.lines.iter().enumerate() {
      let clean = line.trim_end_matches(|c: char| c.is_whitespace());

      let r = index;
      if r >= h {
        break;
      }
      let goto = cursor::Goto(1, r as u16 + 1);

      if !clean.is_empty() || self.alt_background_color.is_some() {
        let (fg, reset_fg) = if let Some(ref dim_fg) = self.dim_color {
          (
            format!("{}", color::Fg(&**dim_fg)),
            format!("{}", color::Fg(color::Reset)),
          )
        } else {
          ("".to_string(), "".to_string())
        };

        let line_vis_width = self.state.line_widths[index];
        let trimmed_line = if line_vis_width > w {
          slice_line_to_width(line, w)
        } else {
          line.to_string()
        };

        let bg_color = if self.alt_background_color.is_some() {
          let bg = if index % 2 == 0 {
            &self.background_color
          } else {
            self.alt_background_color.as_ref().unwrap()
          };
          Some(&**bg)
        } else {
          None
        };

        let final_line = style_ansi_line(&trimmed_line, self.faint, bg_color);
        write!(
          buf,
          "{goto}{fg}{text}{reset_fg}",
          goto = goto,
          fg = fg,
          text = final_line,
          reset_fg = reset_fg
        )
        .unwrap();
      }
    }

    let selected = self.matches.get(self.skip);

    // Pass 1: Render match texts
    for mat in self.matches.iter() {
      let chosen_hint = self.chosen.iter().any(|(hint, _)| hint == mat.text);

      let selected_color = if chosen_hint {
        &self.multi_foreground_color
      } else if selected == Some(mat) {
        &self.select_foreground_color
      } else {
        &self.foreground_color
      };
      let selected_background_color = if chosen_hint {
        &self.multi_background_color
      } else if selected == Some(mat) {
        &self.select_background_color
      } else {
        &self.background_color
      };

      let screen_x = mat.visual_x;
      let screen_y = mat.y as usize;

      if screen_y >= h || screen_x >= w {
        continue;
      }

      let text = self.make_hint_text(mat.text);
      let max_row_width = w - screen_x;
      let raw_width = mat.text.width();

      if screen_x + raw_width <= w {
        // Single-row match: clamp to remaining row width so contrast brackets never wrap
        let clamped_text = if text.width() > max_row_width {
          slice_line_to_width(&text, max_row_width)
        } else {
          text
        };
        write!(
          buf,
          "{goto}{background}{foregroud}{text}{resetf}{resetb}",
          goto = cursor::Goto(screen_x as u16 + 1, screen_y as u16 + 1),
          foregroud = color::Fg(&**selected_color),
          background = color::Bg(&**selected_background_color),
          resetf = color::Fg(color::Reset),
          resetb = color::Bg(color::Reset),
          text = &clamped_text
        )
        .unwrap();
      } else {
        // Multi-row wrapped match: render row-by-row without scrolling past bottom row h
        let mut remaining = text.as_str();
        let mut cur_x = screen_x;
        let mut cur_y = screen_y;
        while !remaining.is_empty() && cur_y < h {
          let avail = w.saturating_sub(cur_x);
          if avail == 0 {
            break;
          }
          let chunk = slice_line_to_width(remaining, avail);
          if chunk.is_empty() {
            break;
          }
          write!(
            buf,
            "{goto}{background}{foregroud}{text}{resetf}{resetb}",
            goto = cursor::Goto(cur_x as u16 + 1, cur_y as u16 + 1),
            foregroud = color::Fg(&**selected_color),
            background = color::Bg(&**selected_background_color),
            resetf = color::Fg(color::Reset),
            resetb = color::Bg(color::Reset),
            text = &chunk
          )
          .unwrap();
          remaining = &remaining[chunk.len()..];
          cur_x = 0;
          cur_y += 1;
        }
      }
    }

    // Pass 2: Render hints on top of match texts
    let mut last_hint_row: Option<usize> = None;
    let mut last_hint_end_x: usize = 0;

    for mat in self.matches.iter() {
      if let Some(ref hint) = mat.hint {
        let visual_offset = mat.visual_x;
        let hint_screen_y = mat.y as usize;

        if hint_screen_y >= h || visual_offset >= w {
          continue;
        }

        let match_text = self.make_hint_text(mat.text);
        let hint_text = self.make_hint_text(hint.as_str());
        let match_width = match_text.width() as i16;
        let hint_width = hint_text.width() as i16;

        let extra_position: i16 = match self.position {
          "right" => match_width - hint_width,
          "off_left" => -hint_width,
          "off_right" => match_width,
          _ => 0,
        };

        let max_x = w.saturating_sub(hint_width as usize);
        let mut hint_screen_x = std::cmp::max(visual_offset as i16 + extra_position, 0) as usize;
        if hint_screen_x > max_x {
          hint_screen_x = max_x;
        }

        // Prevent off_left hint from colliding with the previous hint on the same row
        if self.position == "off_left" && last_hint_row == Some(hint_screen_y) && hint_screen_x < last_hint_end_x {
          hint_screen_x = std::cmp::min(visual_offset, max_x);
        }

        last_hint_row = Some(hint_screen_y);
        last_hint_end_x = hint_screen_x + hint_width as usize;

        write!(
          buf,
          "{goto}{background}{foregroud}{text}{resetf}{resetb}",
          goto = cursor::Goto(hint_screen_x as u16 + 1, hint_screen_y as u16 + 1),
          foregroud = color::Fg(&*self.hint_foreground_color),
          background = color::Bg(&*self.hint_background_color),
          resetf = color::Fg(color::Reset),
          resetb = color::Bg(color::Reset),
          text = &hint_text
        )
        .unwrap();

        if !typed_hint.is_empty() && hint.starts_with(typed_hint) {
          let typed_x = hint_screen_x + if self.contrast { 1 } else { 0 };
          if typed_x < w {
            let clamped_typed = slice_line_to_width(typed_hint, w - typed_x);
            write!(
              buf,
              "{goto}{background}{foregroud}{text}{resetf}{resetb}",
              goto = cursor::Goto(typed_x as u16 + 1, hint_screen_y as u16 + 1),
              foregroud = color::Fg(&*self.multi_foreground_color),
              background = color::Bg(&*self.multi_background_color),
              resetf = color::Fg(color::Reset),
              resetb = color::Bg(color::Reset),
              text = &clamped_typed
            )
            .unwrap();
          }
        }
      }
    }

    stdout.write_all(&buf).unwrap();
    stdout.flush().unwrap();
  }

  fn listen(&mut self, stdin: &mut dyn Read, stdout: &mut dyn Write) -> CaptureEvent {
    if self.matches.is_empty() {
      return CaptureEvent::Exit;
    }

    let mut typed_hint: String = "".to_owned();
    let longest_hint = self
      .matches
      .iter()
      .filter_map(|m| m.hint.as_ref())
      .max_by(|x, y| x.len().cmp(&y.len()))
      .cloned()
      .unwrap_or_default();

    if longest_hint.is_empty() {
      return CaptureEvent::Exit;
    }

    self.render(stdout, &typed_hint);

    let mut keys = stdin.keys();
    while let Some(key) = keys.next() {
      match key {
        Ok(key) => {
          match key {
            Key::Esc => {
              if self.multi && !typed_hint.is_empty() {
                typed_hint.clear();
              } else {
                break;
              }
            }
            Key::Up => {
              self.prev();
            }
            Key::Down => {
              self.next();
            }
            Key::Left => {
              self.prev();
            }
            Key::Right => {
              self.next();
            }
            Key::Backspace => {
              typed_hint.pop();
            }
            Key::Char(ch) => {
              match ch {
                '\n' => match self.matches.get(self.skip).map(|m| m.text) {
                  Some(text) => {
                    self.toggle_or_choose(text, false);

                    if !self.multi {
                      return CaptureEvent::Hint;
                    }
                  }
                  _ => panic!("Match not found?"),
                },
                ' ' => {
                  if self.multi {
                    // Finalize the multi selection
                    return CaptureEvent::Hint;
                  } else {
                    // Enable the multi selection
                    self.multi = true;
                  }
                }
                key => {
                  let key = key.to_string();
                  let lower_key = key.to_lowercase();

                  typed_hint.push_str(lower_key.as_str());

                  let selection = self
                    .matches
                    .iter()
                    .find(|mat| mat.hint.as_deref() == Some(typed_hint.as_str()))
                    .map(|mat| mat.text);

                  match selection {
                    Some(text) => {
                      self.toggle_or_choose(text, key != lower_key);

                      if self.multi {
                        typed_hint.clear();
                      } else {
                        return CaptureEvent::Hint;
                      }
                    }
                    None => {
                      if !self.multi && typed_hint.len() >= longest_hint.len() {
                        break;
                      }
                    }
                  }
                }
              }
            }
            _ => {
              // Unknown key
            }
          }
        }
        Err(err) => panic!("{}", err),
      }

      self.render(stdout, &typed_hint);
    }

    CaptureEvent::Exit
  }

  pub fn present(&mut self) -> Vec<(String, bool)> {
    let mut stdin = termion::get_tty().unwrap();
    let mut stdout = stdout().into_raw_mode().unwrap().into_alternate_screen().unwrap();

    let hints = match self.listen(&mut stdin, &mut stdout) {
      CaptureEvent::Exit => vec![],
      CaptureEvent::Hint => self.chosen.clone(),
    };

    write!(stdout, "{}", cursor::Show).unwrap();

    hints
  }
}

fn slice_line_to_width(line: &str, max_w: usize) -> String {
  let mut current_width = 0;
  let mut last_end = 0;

  let slice_segment = |segment: &str, base_idx: usize, current_width: &mut usize| -> Option<usize> {
    let mut iter = segment.char_indices().peekable();
    while let Some((byte_offset, ch)) = iter.next() {
      let ch_width = if ch == '\t' {
        8 - (*current_width % 8)
      } else if let Some(&(_, '\u{fe0f}')) = iter.peek() {
        let (fe0f_offset, _) = iter.next().unwrap();
        let end_offset = fe0f_offset + '\u{fe0f}'.len_utf8();
        unicode_width::UnicodeWidthStr::width(&segment[byte_offset..end_offset])
      } else {
        unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0)
      };

      if *current_width + ch_width > max_w {
        return Some(base_idx + byte_offset);
      }
      *current_width += ch_width;
    }
    None
  };

  for mat in state::ANSI_RE.find_iter(line) {
    if let Some(cutoff) = slice_segment(&line[last_end..mat.start()], last_end, &mut current_width) {
      return line[..cutoff].to_string();
    }
    last_end = mat.end();
  }

  if let Some(cutoff) = slice_segment(&line[last_end..], last_end, &mut current_width) {
    return line[..cutoff].to_string();
  }

  line.to_string()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn split(output: &str) -> Vec<&str> {
    output.split("\n").collect::<Vec<&str>>()
  }

  #[test]
  fn hint_text() {
    let lines = split("lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let state = state::State::new(&lines, "abcd", &custom);
    let mut view = View {
      state: &state,
      skip: 0,
      multi: false,
      contrast: false,
      faint: false,
      position: &"",
      matches: vec![],
      select_foreground_color: colors::get_color("default"),
      select_background_color: colors::get_color("default"),
      multi_foreground_color: colors::get_color("default"),
      multi_background_color: colors::get_color("default"),
      foreground_color: colors::get_color("default"),
      background_color: colors::get_color("default"),
      alt_background_color: None,
      dim_color: None,
      hint_background_color: colors::get_color("default"),
      hint_foreground_color: colors::get_color("default"),
      chosen: vec![],
    };

    let result = view.make_hint_text("a");
    assert_eq!(result, "a".to_string());

    view.contrast = true;
    let result = view.make_hint_text("a");
    assert_eq!(result, "[a]".to_string());
  }

  #[test]
  fn test_slice_line_to_width() {
    assert_eq!(slice_line_to_width("hello", 3), "hel");
    assert_eq!(slice_line_to_width("\x1b[31mhello\x1b[m", 3), "\x1b[31mhel");
    assert_eq!(slice_line_to_width("hello \x1b[31mworld\x1b[m", 8), "hello \x1b[31mwo");
    assert_eq!(slice_line_to_width("\tmodified", 12), "\tmodi");
    assert_eq!(slice_line_to_width("a\tmodified", 12), "a\tmodi");
    assert_eq!(
      slice_line_to_width("\x1b]8;id=123;https://example.com\x1b\\hello\x1b]8;;\x1b\\", 3),
      "\x1b]8;id=123;https://example.com\x1b\\hel"
    );
    assert_eq!(slice_line_to_width("\x1b(Bhello", 3), "\x1b(Bhel");
    assert_eq!(slice_line_to_width("⚠️bc", 2), "⚠️");
    assert_eq!(slice_line_to_width("⚠️bc", 1), "");
  }

  #[test]
  fn test_listen_with_none_hints() {
    let lines = split("lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let state = state::State::new(&lines, "abcd", &custom);
    let mut view = View {
      state: &state,
      skip: 0,
      multi: false,
      contrast: false,
      faint: false,
      position: &"",
      matches: vec![state::Match {
        x: 0,
        y: 0,
        visual_x: 0,
        pattern: "test",
        text: "lorem",
        hint: None,
      }],
      select_foreground_color: colors::get_color("default"),
      select_background_color: colors::get_color("default"),
      multi_foreground_color: colors::get_color("default"),
      multi_background_color: colors::get_color("default"),
      foreground_color: colors::get_color("default"),
      background_color: colors::get_color("default"),
      alt_background_color: None,
      dim_color: None,
      hint_background_color: colors::get_color("default"),
      hint_foreground_color: colors::get_color("default"),
      chosen: vec![],
    };

    let mut stdin = std::io::empty();
    let mut stdout = Vec::new();
    let result = view.listen(&mut stdin, &mut stdout);
    assert!(matches!(result, CaptureEvent::Exit));
  }

  #[test]
  fn test_style_ansi_line() {
    // Faint mode restores after \x1b[0m, \x1b[m, \x1b[22m, compound \x1b[0;32m, and dims \x1b[1m bold
    assert_eq!(
      style_ansi_line("\x1b[1mBold\x1b[0m plain \x1b[0;32mgreen\x1b[22m end", true, None),
      "\x1b[2m\x1b[1m\x1b[22;2mBold\x1b[0m\x1b[2m plain \x1b[0;32m\x1b[2mgreen\x1b[22m\x1b[2m end\x1b[22m"
    );

    // 24-bit RGB parameters (like 38;2;0;1;22) are not mistaken for SGR 0, 1, or 22
    assert_eq!(
      style_ansi_line("\x1b[38;2;0;1;22mtext\x1b[39m", true, None),
      "\x1b[2m\x1b[38;2;0;1;22mtext\x1b[39m\x1b[22m"
    );

    // Background color restores after \x1b[0m, \x1b[49m, and emits \x1b[K BCE before reset
    let bg = colors::get_color("black");
    let bg_seq = format!("{}", color::Bg(&*bg));
    let bg_reset = format!("{}", color::Bg(color::Reset));
    assert_eq!(
      style_ansi_line("hello \x1b[44mblue\x1b[49m world", false, Some(&*bg)),
      format!(
        "{bg}hello \x1b[44mblue\x1b[49m{bg} world\x1b[K{reset}",
        bg = bg_seq,
        reset = bg_reset
      )
    );

    // Tabs expand to 8-col tab stops
    assert_eq!(style_ansi_line("a\tb", true, None), "\x1b[2ma       b\x1b[22m");
  }

  #[test]
  fn test_empty_reverse_and_multi_toggle() {
    let lines = split("no matches here");
    let custom = [].to_vec();
    let state = state::State::new(&lines, "abcd", &custom);
    let mut view = View::new(
      &state,
      true,
      true,
      false,
      false,
      false,
      "left",
      colors::get_color("default"),
      colors::get_color("default"),
      colors::get_color("default"),
      colors::get_color("default"),
      colors::get_color("default"),
      colors::get_color("default"),
      None,
      None,
      colors::get_color("default"),
      colors::get_color("default"),
    );
    assert_eq!(view.skip, 0);
    view.next();
    assert_eq!(view.skip, 0);

    view.toggle_or_choose("item1", false);
    assert_eq!(view.chosen.len(), 1);
    view.toggle_or_choose("item1", false);
    assert_eq!(view.chosen.len(), 0);
  }

  #[test]
  fn test_fast_two_char_hint_and_buffered_render() {
    let lines = split("127.0.0.1 127.0.0.2 127.0.0.3 127.0.0.4 127.0.0.5");
    let custom = [].to_vec();
    let state = state::State::new(&lines, "abcd", &custom);
    let mut view = View::new(
      &state,
      false,
      false,
      false,
      false,
      false,
      "left",
      colors::get_color("default"),
      colors::get_color("default"),
      colors::get_color("default"),
      colors::get_color("default"),
      colors::get_color("default"),
      colors::get_color("default"),
      None,
      None,
      colors::get_color("default"),
      colors::get_color("default"),
    );

    // With alphabet "abcd" and 5 matches, hints are ["a", "b", "c", "da", "db"].
    // Sending both characters "db" in a single buffer tests that no keystrokes are dropped between iterations.
    let mut stdin = "db".as_bytes();
    let mut stdout = Vec::new();
    let result = view.listen(&mut stdin, &mut stdout);
    assert!(!stdout.is_empty());
    match result {
      CaptureEvent::Hint => {
        assert_eq!(view.chosen.len(), 1);
        assert_eq!(view.chosen[0].0, "127.0.0.5");
      }
      CaptureEvent::Exit => panic!("Expected Hint, got Exit"),
    }
  }

  #[test]
  fn test_render_vis_file() {
    if let Ok(content) = std::fs::read_to_string("/usr/local/google/home/sselcuk/vis.txt") {
      let lines = split(&content);
      let custom = [].to_vec();
      let state = state::State::new(&lines, "qwerty", &custom);
      let mut view = View::new(
        &state,
        false,
        false,
        false,
        false,
        false,
        "left",
        colors::get_color("default"),
        colors::get_color("default"),
        colors::get_color("default"),
        colors::get_color("default"),
        colors::get_color("default"),
        colors::get_color("default"),
        None,
        None,
        colors::get_color("default"),
        colors::get_color("default"),
      );

      let b828bec_match = view.matches.iter().find(|m| m.text == "b828bec");
      assert!(b828bec_match.is_some(), "b828bec must be matched");
      assert!(b828bec_match.unwrap().hint.is_some(), "b828bec must get a hint");

      // Verify no phantom merged match like "eb828bec" exists
      let phantom_match = view.matches.iter().find(|m| m.text == "eb828bec");
      assert!(
        phantom_match.is_none(),
        "eb828bec should not be matched across line boundary"
      );

      let mut stdin = std::io::empty();
      let mut stdout = Vec::new();
      let result = view.listen(&mut stdin, &mut stdout);
      assert!(matches!(result, CaptureEvent::Exit));
      assert!(!stdout.is_empty());
    }
  }
}

fn style_ansi_line(s: &str, faint: bool, bg_color: Option<&dyn color::Color>) -> String {
  let has_tabs = s.contains('\t');
  if !has_tabs && !faint && bg_color.is_none() {
    return s.to_string();
  }

  let mut prefix = String::new();
  let mut suffix = String::new();

  if faint {
    prefix.push_str("\x1b[2m");
    suffix.push_str("\x1b[22m");
  }

  let bg_seq = bg_color.map(|bg| format!("{}", color::Bg(bg)));
  if let Some(ref bg) = bg_seq {
    prefix.push_str(bg);
    suffix.push_str("\x1b[K");
    suffix.push_str(format!("{}", color::Bg(color::Reset)).as_str());
  }

  let mut result = String::with_capacity(s.len() + prefix.len() + suffix.len() + 16);
  result.push_str(&prefix);

  let mut col = 0;
  let push_segment = |segment: &str, out: &mut String, col: &mut usize| {
    if has_tabs {
      for (i, part) in segment.split('\t').enumerate() {
        if i > 0 {
          let spaces = 8 - (*col % 8);
          for _ in 0..spaces {
            out.push(' ');
          }
          *col += spaces;
        }
        out.push_str(part);
        *col += unicode_width::UnicodeWidthStr::width(part);
      }
    } else {
      out.push_str(segment);
    }
  };

  let mut last_end = 0;
  for mat in state::ANSI_RE.find_iter(s) {
    push_segment(&s[last_end..mat.start()], &mut result, &mut col);
    let seq = mat.as_str();
    result.push_str(seq);

    if (faint || bg_seq.is_some()) && seq.starts_with("\x1b[") && seq.ends_with('m') {
      let params = &seq[2..seq.len() - 1];
      let mut restore_faint = false;
      let mut clear_bold_first = false;
      let mut restore_bg = false;

      let tokens: Vec<&str> = params.split(';').collect();
      let mut i = 0;
      while i < tokens.len() {
        let tok = tokens[i];
        let code = tok.split(':').next().unwrap_or("");
        if code == "38" || code == "48" || code == "58" {
          if !tok.contains(':') {
            match tokens.get(i + 1).copied() {
              Some("5") => {
                i += 3;
                continue;
              }
              Some("2") => {
                i += 5;
                continue;
              }
              _ => {}
            }
          }
        } else {
          match code {
            "" | "0" => {
              restore_faint = faint;
              restore_bg = bg_seq.is_some();
            }
            "1" => {
              if faint {
                restore_faint = true;
                clear_bold_first = true;
              }
            }
            "22" => {
              if faint {
                restore_faint = true;
              }
            }
            "49" => {
              if bg_seq.is_some() {
                restore_bg = true;
              }
            }
            _ => {}
          }
        }
        i += 1;
      }

      if clear_bold_first {
        result.push_str("\x1b[22;2m");
      } else if restore_faint {
        result.push_str("\x1b[2m");
      }
      if restore_bg {
        if let Some(ref bg) = bg_seq {
          result.push_str(bg);
        }
      }
    }

    last_end = mat.end();
  }

  push_segment(&s[last_end..], &mut result, &mut col);
  result.push_str(&suffix);
  result
}
