use super::*;
use std::char;
use std::io::{stdout, Read, Write};
use termion::async_stdin;
use termion::event::Key;
use termion::input::TermRead;
use termion::raw::IntoRawMode;
use termion::screen::IntoAlternateScreen;
use termion::{color, cursor};

use unicode_width::UnicodeWidthStr;
use unicode_width::UnicodeWidthChar;

pub struct View<'a> {
  state: &'a state::State<'a>,
  skip: usize,
  multi: bool,
  contrast: bool,
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
    let skip = if reverse { matches.len() - 1 } else { 0 };

    View {
      state,
      skip,
      multi,
      contrast,
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
    if self.skip < self.matches.len() - 1 {
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

  fn render(&self, stdout: &mut dyn Write, typed_hint: &str) -> () {
    write!(stdout, "{}", cursor::Hide).unwrap();

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
          (format!("{}", color::Fg(&**dim_fg)), format!("{}", color::Fg(color::Reset)))
        } else {
          ("".to_string(), "".to_string())
        };

        if r == h - 1 {
          // BOTTOM ROW: Print text as-is, trimmed to w-1, with NO trailing padding!
          let line_vis_width = state::visual_width(line);
          let wrap_w = if w > 1 { w - 1 } else { w };
          
          let trimmed_line = if line_vis_width > wrap_w {
            slice_line_to_width(line, wrap_w)
          } else {
            line.to_string()
          };
          
          if let Some(ref alt_bg) = self.alt_background_color {
            let bg = if index % 2 == 0 { &self.background_color } else { alt_bg };
            print!("{goto}{bg}{fg}{text}{reset_fg}{resetb}", goto = goto, bg = color::Bg(&**bg), fg = fg, text = trimmed_line, reset_fg = reset_fg, resetb = color::Bg(color::Reset));
          } else {
            print!("{goto}{fg}{text}{reset_fg}", goto = goto, fg = fg, text = trimmed_line, reset_fg = reset_fg);
          }
        } else {
          // NORMAL ROW: Pad to w-1 (or wrap_w)
          if let Some(ref alt_bg) = self.alt_background_color {
            let bg = if index % 2 == 0 { &self.background_color } else { alt_bg };
            let line_vis_width = state::visual_width(line);
            let wrap_w = if w > 1 { w - 1 } else { w };
            let padding_len = if line_vis_width < wrap_w { wrap_w - line_vis_width } else { 0 };
            let padded = format!("{}{}", line, " ".repeat(padding_len));
            print!("{goto}{bg}{fg}{text}{reset_fg}{resetb}", goto = goto, bg = color::Bg(&**bg), fg = fg, text = padded, reset_fg = reset_fg, resetb = color::Bg(color::Reset));
          } else {
            print!("{goto}{fg}{text}{reset_fg}", goto = goto, fg = fg, text = line, reset_fg = reset_fg);
          }
        }
      }
    }

    let selected = self.matches.get(self.skip);

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

      let line = &self.state.lines[mat.y as usize];
      let prefix: String = line.chars().take(mat.x as usize).collect();
      
      let visual_offset = state::visual_width(&prefix);
      
      let screen_x = visual_offset;
      let screen_y = mat.y as usize;

      if screen_y >= h {
        continue;
      }

      let text = self.make_hint_text(mat.text);

      print!(
        "{goto}{background}{foregroud}{text}{resetf}{resetb}",
        goto = cursor::Goto(screen_x as u16 + 1, screen_y as u16 + 1),
        foregroud = color::Fg(&**selected_color),
        background = color::Bg(&**selected_background_color),
        resetf = color::Fg(color::Reset),
        resetb = color::Bg(color::Reset),
        text = &text
      );

      if let Some(ref hint) = mat.hint {
        let extra_position: i16 = match self.position {
          "right" => text.width_cjk() as i16 - hint.len() as i16,
          "off_left" => 0 - hint.len() as i16 - if self.contrast { 2 } else { 0 },
          "off_right" => text.width_cjk() as i16,
          _ => 0,
        };

        let text = self.make_hint_text(hint.as_str());
        let final_position = std::cmp::max(visual_offset as i16 + extra_position, 0) as usize;

        let hint_screen_x = final_position;
        let hint_screen_y = mat.y as usize;

        if hint_screen_y >= h {
          continue;
        }

        print!(
          "{goto}{background}{foregroud}{text}{resetf}{resetb}",
          goto = cursor::Goto(hint_screen_x as u16 + 1, hint_screen_y as u16 + 1),
          foregroud = color::Fg(&*self.hint_foreground_color),
          background = color::Bg(&*self.hint_background_color),
          resetf = color::Fg(color::Reset),
          resetb = color::Bg(color::Reset),
          text = &text
        );

        if hint.starts_with(typed_hint) {
          print!(
            "{goto}{background}{foregroud}{text}{resetf}{resetb}",
            goto = cursor::Goto(hint_screen_x as u16 + 1, hint_screen_y as u16 + 1),
            foregroud = color::Fg(&*self.multi_foreground_color),
            background = color::Bg(&*self.multi_background_color),
            resetf = color::Fg(color::Reset),
            resetb = color::Bg(color::Reset),
            text = &typed_hint
          );
        }
      }
    }

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
      .filter_map(|m| m.hint.clone())
      .max_by(|x, y| x.len().cmp(&y.len()))
      .unwrap()
      .clone();

    self.render(stdout, &typed_hint);

    loop {
      match stdin.keys().next() {
        Some(key) => {
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
                    '\n' => match self.matches.iter().enumerate().find(|&h| h.0 == self.skip) {
                      Some(hm) => {
                        self.chosen.push((hm.1.text.to_string(), false));

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

                      let selection = self.matches.iter().find(|mat| mat.hint == Some(typed_hint.clone()));

                      match selection {
                        Some(mat) => {
                          self.chosen.push((mat.text.to_string(), key != lower_key));

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

          stdin.keys().for_each(|_| { /* Skip the rest of stdin buffer */ })
        }
        _ => {
          if !termion::is_tty(&std::io::stdin()) {
            break;
          }
          std::thread::sleep(std::time::Duration::from_millis(50));
          continue; // don't render again if nothing new to show
        }
      }

      self.render(stdout, &typed_hint);
    }

    CaptureEvent::Exit
  }

  pub fn present(&mut self) -> Vec<(String, bool)> {
    let mut stdin = async_stdin();
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
  let mut sliced = String::new();
  let mut current_width = 0;
  let mut in_escape = false;
  
  for ch in line.chars() {
    if in_escape {
      sliced.push(ch);
      if ch.is_ascii_alphabetic() {
        in_escape = false;
      }
      continue;
    }
    
    if ch == '\x1b' {
      in_escape = true;
      sliced.push(ch);
      continue;
    }
    
    let ch_width = if ch == '\t' {
      8 - (current_width % 8)
    } else {
      unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0)
    };
    
    if current_width + ch_width > max_w {
      break;
    }
    sliced.push(ch);
    current_width += ch_width;
  }
  sliced
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
  }
}
