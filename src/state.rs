use regex::Regex;
use std::collections::HashMap;
use std::fmt;

lazy_static! {
  static ref ANSI_RE: Regex = Regex::new(r"(\x1b|\\033|\\e)\[[0-9;:?]*[a-zA-Z]").unwrap();
  static ref COMPILED_EXCLUDE_PATTERNS: Vec<(&'static str, Regex)> = EXCLUDE_PATTERNS
    .iter()
    .map(|tuple| (tuple.0, Regex::new(tuple.1).unwrap()))
    .collect();
  static ref COMPILED_PATTERNS: Vec<(&'static str, Regex)> = PATTERNS
    .iter()
    .map(|tuple| (tuple.0, Regex::new(tuple.1).unwrap()))
    .collect();
}

pub fn visual_width(s: &str) -> usize {
  let stripped = ANSI_RE.replace_all(s, "");
  let mut width = 0;
  for ch in stripped.chars() {
    if ch == '\t' {
      width += 8 - (width % 8);
    } else {
      width += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
    }
  }
  width
}

const EXCLUDE_PATTERNS: [(&'static str, &'static str); 0] = [];

const PATTERNS: [(&'static str, &'static str); 15] = [
  ("markdown_url", r"\[[^]]*\]\(([^)]+)\)"),
  ("url", r"(?P<match>(https?://|git@|git://|ssh://|ftp://|file:///)[^ \n]+)"),
  (
    "diff_summary",
    r"diff --git a/([.\w\-@~\[\]]+?/[.\w\-@\[\]]++) b/([.\w\-@~\[\]]+?/[.\w\-@\[\]]++)",
  ),
  ("diff_a", r"--- a/([^ \n]+)"),
  ("diff_b", r"\+\+\+ b/([^ \n]+)"),
  ("docker", r"sha256:([0-9a-f]{64})"),
  ("path", r"(?P<match>([.\w\-@$~\[\]]+)?(/[.\w\-@$\[\]]+)+)"),
  ("color", r"#[0-9a-fA-F]{6}"),
  ("uid", r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"),
  ("ipfs", r"Qm[0-9a-zA-Z]{44}"),
  ("sha", r"[0-9a-f]{7,40}"),
  ("ip", r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}"),
  ("ipv6", r"[a-fA-F0-9:]+:+[a-fA-F0-9:]+[%\w\d]+"),
  ("address", r"0x[0-9a-fA-F]+"),
  ("number", r"[0-9]{4,}"),
];

#[derive(Clone)]
pub struct Match<'a> {
  pub x: i32,
  pub y: i32,
  pub visual_x: usize,
  pub pattern: &'a str,
  pub text: &'a str,
  pub hint: Option<String>,
}

impl<'a> fmt::Debug for Match<'a> {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(
      f,
      "Match {{ x: {}, y: {}, visual_x: {}, pattern: {}, text: {}, hint: <{}> }}",
      self.x,
      self.y,
      self.visual_x,
      self.pattern,
      self.text,
      self.hint.clone().unwrap_or("<undefined>".to_string())
    )
  }
}

impl<'a> PartialEq for Match<'a> {
  fn eq(&self, other: &Match) -> bool {
    self.x == other.x && self.y == other.y
  }
}

pub struct State<'a> {
  pub lines: &'a Vec<&'a str>,
  pub line_widths: Vec<usize>,
  pub j: String,
  pub map: Vec<(i32, i32)>,
  alphabet: &'a str,
  regexp: &'a Vec<&'a str>,
}

impl<'a> State<'a> {
  pub fn new(lines: &'a Vec<&'a str>, alphabet: &'a str, regexp: &'a Vec<&'a str>) -> State<'a> {
    let total_bytes: usize = lines.iter().map(|l| l.len()).sum();
    let mut j = String::with_capacity(total_bytes + lines.len());
    let mut map = Vec::with_capacity(total_bytes + lines.len());
    
    let line_widths: Vec<usize> = lines.iter().map(|l| visual_width(l)).collect();
    let usable_width = line_widths.iter().max().cloned().unwrap_or(0);

    for (v_line_index, v_line) in lines.iter().enumerate() {
      let is_wrapped = usable_width > 0 && line_widths[v_line_index] >= usable_width && v_line_index < lines.len() - 1;
      
      let ansi_spans = ANSI_RE.find_iter(v_line).map(|m| (m.start(), m.end())).collect::<Vec<_>>();
      
      let mut char_count = 0;
      let mut active_span_idx = 0;

      for (byte_index, ch) in v_line.char_indices() {
        while active_span_idx < ansi_spans.len() && byte_index >= ansi_spans[active_span_idx].1 {
          active_span_idx += 1;
        }

        let in_ansi = active_span_idx < ansi_spans.len() && byte_index >= ansi_spans[active_span_idx].0;
        
        if !in_ansi {
          let bytes = ch.len_utf8();
          for _ in 0..bytes {
            map.push((v_line_index as i32, char_count as i32));
          }
          j.push(ch);
        }
        char_count += 1;
      }
      
      if !is_wrapped {
        j.push('\n');
        map.push((v_line_index as i32, v_line.chars().count() as i32));
      }
    }

    State {
      lines,
      line_widths,
      j,
      map,
      alphabet,
      regexp,
    }
  }

  pub fn matches(&'a self, reverse: bool, unique: bool) -> Vec<Match<'a>> {
    let mut matches = Vec::new();

    let custom_patterns = self
      .regexp
      .iter()
      .map(|regexp| ("custom", Regex::new(regexp).expect("Invalid custom regexp")))
      .collect::<Vec<_>>();

    let all_patterns: Vec<(&str, Regex)> = COMPILED_EXCLUDE_PATTERNS
      .iter()
      .map(|(name, re)| (*name, re.clone()))
      .chain(custom_patterns.into_iter())
      .chain(COMPILED_PATTERNS.iter().map(|(name, re)| (*name, re.clone())))
      .collect();

    struct RawMatch<'a> {
      start: usize,
      end: usize,
      pattern_name: &'a str,
      pattern: Regex,
      text: &'a str,
      priority: usize,
    }

    let mut raw_matches = Vec::new();
    for (priority, (name, pattern)) in all_patterns.iter().enumerate() {
      for m in pattern.find_iter(&self.j) {
        raw_matches.push(RawMatch {
          start: m.start(),
          end: m.end(),
          pattern_name: name,
          pattern: pattern.clone(),
          text: m.as_str(),
          priority,
        });
      }
    }

    raw_matches.sort_by(|a, b| {
      a.start.cmp(&b.start).then(a.priority.cmp(&b.priority))
    });

    let mut last_end = 0;
    for rm in raw_matches {
      if rm.start < last_end {
        continue;
      }

      if let Some(captures) = rm.pattern.captures(rm.text) {
        let captures: Vec<(&str, usize)> = if let Some(capture) = captures.name("match") {
          [(capture.as_str(), capture.start())].to_vec()
        } else if captures.len() > 1 {
          captures
            .iter()
            .skip(1)
            .filter_map(|capture| capture)
            .map(|capture| (capture.as_str(), capture.start()))
            .collect::<Vec<(&str, usize)>>()
        } else {
          [(rm.text, 0)].to_vec()
        };

        if rm.pattern_name != "bash" {
          for (subtext, substart) in captures.iter() {
            let j_match_start = rm.start + *substart;

            if j_match_start < self.map.len() {
              let (v_line, v_char) = self.map[j_match_start];
              let line = self.lines[v_line as usize];
              let prefix: String = line.chars().take(v_char as usize).collect();
              let visual_x = visual_width(&prefix);

              matches.push(Match {
                x: v_char,
                y: v_line,
                visual_x,
                pattern: rm.pattern_name,
                text: subtext,
                hint: None,
              });
            }
          }
        }

        last_end = rm.end;
      }
    }

    let alphabet = super::alphabets::get_alphabet(self.alphabet);
    let mut hints = alphabet.hints(matches.len());

    if !reverse {
      hints.reverse();
    } else {
      matches.reverse();
      hints.reverse();
    }

    if unique {
      let mut previous: HashMap<&str, String> = HashMap::new();

      for mat in &mut matches {
        if let Some(previous_hint) = previous.get(mat.text) {
          mat.hint = Some(previous_hint.clone());
        } else if let Some(hint) = hints.pop() {
          mat.hint = Some(hint.to_string().clone());
          previous.insert(mat.text, hint.to_string().clone());
        }
      }
    } else {
      for mat in &mut matches {
        if let Some(hint) = hints.pop() {
          mat.hint = Some(hint.to_string().clone());
        }
      }
    }

    if reverse {
      matches.reverse();
    }

    matches
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn split(output: &str) -> Vec<&str> {
    output.split("\n").collect::<Vec<&str>>()
  }

  #[test]
  fn match_reverse() {
    let lines = split("lorem 127.0.0.1 lorem 255.255.255.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.first().unwrap().hint.clone().unwrap(), "a");
    assert_eq!(results.last().unwrap().hint.clone().unwrap(), "c");
  }

  #[test]
  fn match_unique() {
    let lines = split("lorem 127.0.0.1 lorem 255.255.255.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, true);

    assert_eq!(results.len(), 3);
    assert_eq!(results.first().unwrap().hint.clone().unwrap(), "a");
    assert_eq!(results.last().unwrap().hint.clone().unwrap(), "a");
  }

  #[test]
  fn match_docker() {
    let lines = split("latest sha256:30557a29d5abc51e5f1d5b472e79b7e296f595abcf19fe6b9199dbbc809c6ff4 20 hours ago");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(
      results.get(0).unwrap().text,
      "30557a29d5abc51e5f1d5b472e79b7e296f595abcf19fe6b9199dbbc809c6ff4"
    );
  }

  #[test]
  fn match_bash() {
    let lines = split("path: \u{1b}[32m/var/log/nginx.log\u{1b}[m\npath: \u{1b}[32mtest/log/nginx-2.log:32\u{1b}[m folder/.nginx@4df2.log");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text, "/var/log/nginx.log");
    assert_eq!(results.get(1).unwrap().text, "test/log/nginx-2.log");
    assert_eq!(results.get(2).unwrap().text, "folder/.nginx@4df2.log");
  }

  #[test]
  fn match_paths() {
    let lines = split("Lorem /tmp/foo/bar_lol, lorem\n Lorem /var/log/boot-strap.log lorem ../log/kern.log lorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text.clone(), "/tmp/foo/bar_lol");
    assert_eq!(results.get(1).unwrap().text.clone(), "/var/log/boot-strap.log");
    assert_eq!(results.get(2).unwrap().text.clone(), "../log/kern.log");
  }

  #[test]
  fn match_routes() {
    let lines = split("Lorem /app/routes/$routeId/$objectId, lorem\n Lorem /app/routes/$sectionId");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().text.clone(), "/app/routes/$routeId/$objectId");
    assert_eq!(results.get(1).unwrap().text.clone(), "/app/routes/$sectionId");
  }

  #[test]
  fn match_home() {
    let lines = split("Lorem ~/.gnu/.config.txt, lorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "~/.gnu/.config.txt");
  }

  #[test]
  fn match_slugs() {
    let lines = split("Lorem dev/api/[slug]/foo, lorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "dev/api/[slug]/foo");
  }

  #[test]
  fn match_uids() {
    let lines = split("Lorem ipsum 123e4567-e89b-12d3-a456-426655440000 lorem\n Lorem lorem lorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 1);
  }

  #[test]
  fn match_shas() {
    let lines = split("Lorem fd70b5695 5246ddf f924213 lorem\n Lorem 973113963b491874ab2e372ee60d4b4cb75f717c lorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "fd70b5695");
    assert_eq!(results.get(1).unwrap().text.clone(), "5246ddf");
    assert_eq!(results.get(2).unwrap().text.clone(), "f924213");
    assert_eq!(
      results.get(3).unwrap().text.clone(),
      "973113963b491874ab2e372ee60d4b4cb75f717c"
    );
  }

  #[test]
  fn match_ips() {
    let lines = split("Lorem ipsum 127.0.0.1 lorem\n Lorem 255.255.10.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text.clone(), "127.0.0.1");
    assert_eq!(results.get(1).unwrap().text.clone(), "255.255.10.255");
    assert_eq!(results.get(2).unwrap().text.clone(), "127.0.0.1");
  }

  #[test]
  fn match_ipv6s() {
    let lines = split("Lorem ipsum fe80::2:202:fe4 lorem\n Lorem 2001:67c:670:202:7ba8:5e41:1591:d723 lorem fe80::2:1 lorem ipsum fe80:22:312:fe::1%eth0");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "fe80::2:202:fe4");
    assert_eq!(
      results.get(1).unwrap().text.clone(),
      "2001:67c:670:202:7ba8:5e41:1591:d723"
    );
    assert_eq!(results.get(2).unwrap().text.clone(), "fe80::2:1");
    assert_eq!(results.get(3).unwrap().text.clone(), "fe80:22:312:fe::1%eth0");
  }

  #[test]
  fn match_markdown_urls() {
    let lines = split("Lorem ipsum [link](https://github.io?foo=bar) ![](http://cdn.com/img.jpg) lorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().pattern.clone(), "markdown_url");
    assert_eq!(results.get(0).unwrap().text.clone(), "https://github.io?foo=bar");
    assert_eq!(results.get(1).unwrap().pattern.clone(), "markdown_url");
    assert_eq!(results.get(1).unwrap().text.clone(), "http://cdn.com/img.jpg");
  }

  #[test]
  fn match_urls() {
    let lines = split("Lorem ipsum https://www.rust-lang.org/tools lorem\n Lorem ipsumhttps://crates.io lorem https://github.io?foo=bar lorem ssh://github.io");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "https://www.rust-lang.org/tools");
    assert_eq!(results.get(0).unwrap().pattern.clone(), "url");
    assert_eq!(results.get(1).unwrap().text.clone(), "https://crates.io");
    assert_eq!(results.get(1).unwrap().pattern.clone(), "url");
    assert_eq!(results.get(2).unwrap().text.clone(), "https://github.io?foo=bar");
    assert_eq!(results.get(2).unwrap().pattern.clone(), "url");
    assert_eq!(results.get(3).unwrap().text.clone(), "ssh://github.io");
    assert_eq!(results.get(3).unwrap().pattern.clone(), "url");
  }

  #[test]
  fn match_addresses() {
    let lines = split("Lorem 0xfd70b5695 0x5246ddf lorem\n Lorem 0x973113tlorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text.clone(), "0xfd70b5695");
    assert_eq!(results.get(1).unwrap().text.clone(), "0x5246ddf");
    assert_eq!(results.get(2).unwrap().text.clone(), "0x973113");
  }

  #[test]
  fn match_hex_colors() {
    let lines = split("Lorem #fd7b56 lorem #FF00FF\n Lorem #00fF05 lorem #abcd00 lorem #afRR00");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "#fd7b56");
    assert_eq!(results.get(1).unwrap().text.clone(), "#FF00FF");
    assert_eq!(results.get(2).unwrap().text.clone(), "#00fF05");
    assert_eq!(results.get(3).unwrap().text.clone(), "#abcd00");
  }

  #[test]
  fn match_ipfs() {
    let lines = split("Lorem QmRdbNSxDJBXmssAc9fvTtux4duptMvfSGiGuq6yHAQVKQ lorem Qmfoobar");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(
      results.get(0).unwrap().text.clone(),
      "QmRdbNSxDJBXmssAc9fvTtux4duptMvfSGiGuq6yHAQVKQ"
    );
  }

  #[test]
  fn match_process_port() {
    let lines =
      split("Lorem 5695 52463 lorem\n Lorem 973113 lorem 99999 lorem 8888 lorem\n   23456 lorem 5432 lorem 23444");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 8);
  }

  #[test]
  fn match_diff_a() {
    let lines = split("Lorem lorem\n--- a/src/main.rs");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "src/main.rs");
  }

  #[test]
  fn match_diff_b() {
    let lines = split("Lorem lorem\n+++ b/src/main.rs");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "src/main.rs");
  }

  #[test]
  fn match_diff_summary() {
    let lines = split("diff --git a/samples/test1 b/samples/test2");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().text.clone(), "samples/test1");
    assert_eq!(results.get(1).unwrap().text.clone(), "samples/test2");
  }
  #[test]
  fn priority() {
    let lines = split("Lorem [link](http://foo.bar) ipsum CUSTOM-52463 lorem ISSUE-123 lorem\nLorem /var/fd70b569/9999.log 52463 lorem\n Lorem 973113 lorem 123e4567-e89b-12d3-a456-426655440000 lorem 8888 lorem\n  https://crates.io/23456/fd70b569 lorem");
    let custom = ["CUSTOM-[0-9]{4,}", "ISSUE-[0-9]{3}"].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 9);
    assert_eq!(results.get(0).unwrap().text.clone(), "http://foo.bar");
    assert_eq!(results.get(1).unwrap().text.clone(), "CUSTOM-52463");
    assert_eq!(results.get(2).unwrap().text.clone(), "ISSUE-123");
    assert_eq!(results.get(3).unwrap().text.clone(), "/var/fd70b569/9999.log");
    assert_eq!(results.get(4).unwrap().text.clone(), "52463");
    assert_eq!(results.get(5).unwrap().text.clone(), "973113");
    assert_eq!(
      results.get(6).unwrap().text.clone(),
      "123e4567-e89b-12d3-a456-426655440000"
    );
    assert_eq!(results.get(7).unwrap().text.clone(), "8888");
    assert_eq!(results.get(8).unwrap().text.clone(), "https://crates.io/23456/fd70b569");
  }

  #[test]
  fn test_visual_width() {
    assert_eq!(visual_width("hello"), 5);
    assert_eq!(visual_width("\x1b[31mhello\x1b[m"), 5);
    assert_eq!(visual_width("\x1b[1;31mhello\x1b[0m world"), 11);
    assert_eq!(visual_width("    \x1b[31mmodified:\x1b[m   "), 16);
    assert_eq!(visual_width("\t\x1b[31mmodified:\x1b[m    "), 21);
    assert_eq!(visual_width("a\t"), 8);
  }

}
