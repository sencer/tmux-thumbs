use regex::Regex;
use std::collections::HashMap;
use std::fmt;

lazy_static! {
  pub(crate) static ref ANSI_RE: Regex = Regex::new(
    r"\x1b(?:\][^\x07\x1b]*(?:\x07|\x1b\\)|[PX^_][^\x07\x1b]*(?:\x07|\x1b\\)|\[[0-9:;<=>?]*[ -/]*[@-~]|[ -/]+[0-~]|[0-~])"
  )
  .unwrap();
  static ref COMPILED_EXCLUDE_PATTERNS: Vec<(&'static str, Regex)> = EXCLUDE_PATTERNS
    .iter()
    .map(|tuple| (tuple.0, Regex::new(tuple.1).unwrap()))
    .collect();
  static ref COMPILED_PATTERNS: Vec<(&'static str, Regex)> = PATTERNS
    .iter()
    .map(|tuple| (tuple.0, Regex::new(tuple.1).unwrap()))
    .collect();
}

#[cfg(test)]
pub fn visual_width(s: &str) -> usize {
  let mut width = 0;
  let mut last_end = 0;
  for m in ANSI_RE.find_iter(s) {
    width = add_segment_width(&s[last_end..m.start()], width);
    last_end = m.end();
  }
  add_segment_width(&s[last_end..], width)
}

#[cfg(test)]
fn add_segment_width(segment: &str, mut width: usize) -> usize {
  for (i, part) in segment.split('\t').enumerate() {
    if i > 0 {
      width += 8 - (width % 8);
    }
    width += unicode_width::UnicodeWidthStr::width(part);
  }
  width
}

const EXCLUDE_PATTERNS: [(&'static str, &'static str); 0] = [];

const PATTERNS: [(&'static str, &'static str); 15] = [
  ("markdown_url", r"\[[^]]*\]\(([^)]+)\)"),
  (
    "url",
    r#"(?P<match>(https?://|git@|git://|ssh://|ftp://|file:///)[^ \n<>"']+)"#,
  ),
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
  pub map: Vec<(i32, i32, usize)>,
  alphabet: &'a str,
  regexp: &'a Vec<&'a str>,
}

impl<'a> State<'a> {
  pub fn new(lines: &'a Vec<&'a str>, alphabet: &'a str, regexp: &'a Vec<&'a str>) -> State<'a> {
    let total_bytes: usize = lines.iter().map(|l| l.len()).sum();
    let mut j = String::with_capacity(total_bytes + lines.len());
    let mut map = Vec::with_capacity(total_bytes + lines.len());

    struct LineMeta {
      ansi_spans: Vec<(usize, usize)>,
    }

    let mut metas = Vec::with_capacity(lines.len());
    let mut line_widths = Vec::with_capacity(lines.len());

    for v_line in lines.iter() {
      let mut ansi_spans = Vec::new();
      let mut total_width = 0;

      let mut process_segment = |segment: &str| {
        let mut iter = segment.char_indices().peekable();
        while let Some((byte_offset, ch)) = iter.next() {
          let ch_width = if ch == '\t' {
            8 - (total_width % 8)
          } else if let Some(&(_, '\u{fe0f}')) = iter.peek() {
            let (fe0f_offset, _) = iter.next().unwrap();
            let end_offset = fe0f_offset + '\u{fe0f}'.len_utf8();
            unicode_width::UnicodeWidthStr::width(&segment[byte_offset..end_offset])
          } else {
            unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0)
          };
          total_width += ch_width;
        }
      };

      let mut last_end = 0;
      for m in ANSI_RE.find_iter(v_line) {
        process_segment(&v_line[last_end..m.start()]);
        ansi_spans.push((m.start(), m.end()));
        last_end = m.end();
      }
      process_segment(&v_line[last_end..]);

      line_widths.push(total_width);
      metas.push(LineMeta { ansi_spans });
    }

    for (v_line_index, v_line) in lines.iter().enumerate() {
      let meta = &metas[v_line_index];

      let mut char_count = 0;
      let mut visual_col = 0;
      let mut last_end = 0;

      let push_text_segment = |segment: &str,
                               char_count: &mut usize,
                               visual_col: &mut usize,
                               j: &mut String,
                               map: &mut Vec<(i32, i32, usize)>| {
        let mut iter = segment.char_indices().peekable();
        while let Some((byte_offset, ch)) = iter.next() {
          if ch == '\t' {
            let w = 8 - (*visual_col % 8);
            for _ in 0..ch.len_utf8() {
              map.push((v_line_index as i32, *char_count as i32, *visual_col));
            }
            j.push(ch);
            *char_count += 1;
            *visual_col += w;
          } else if let Some(&(fe0f_offset, '\u{fe0f}')) = iter.peek() {
            let end_offset = fe0f_offset + '\u{fe0f}'.len_utf8();
            let w = unicode_width::UnicodeWidthStr::width(&segment[byte_offset..end_offset]);
            for _ in 0..ch.len_utf8() {
              map.push((v_line_index as i32, *char_count as i32, *visual_col));
            }
            j.push(ch);
            *char_count += 1;

            iter.next();
            for _ in 0..'\u{fe0f}'.len_utf8() {
              map.push((v_line_index as i32, *char_count as i32, *visual_col));
            }
            j.push('\u{fe0f}');
            *char_count += 1;
            *visual_col += w;
          } else {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            for _ in 0..ch.len_utf8() {
              map.push((v_line_index as i32, *char_count as i32, *visual_col));
            }
            j.push(ch);
            *char_count += 1;
            *visual_col += w;
          }
        }
      };

      for &(span_start, span_end) in &meta.ansi_spans {
        push_text_segment(
          &v_line[last_end..span_start],
          &mut char_count,
          &mut visual_col,
          &mut j,
          &mut map,
        );
        char_count += v_line[span_start..span_end].chars().count();
        last_end = span_end;
      }
      push_text_segment(&v_line[last_end..], &mut char_count, &mut visual_col, &mut j, &mut map);

      j.push('\n');
      map.push((v_line_index as i32, char_count as i32, visual_col));
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

    let all_patterns: Vec<(&str, &Regex)> = COMPILED_EXCLUDE_PATTERNS
      .iter()
      .map(|(name, re)| (*name, re))
      .chain(custom_patterns.iter().map(|(name, re)| (*name, re)))
      .chain(COMPILED_PATTERNS.iter().map(|(name, re)| (*name, re)))
      .collect();

    struct RawMatch<'a, 'b> {
      start: usize,
      end: usize,
      pattern_name: &'a str,
      pattern: &'b Regex,
      text: &'a str,
      priority: usize,
    }

    let mut raw_matches = Vec::new();
    for (priority, &(name, pattern)) in all_patterns.iter().enumerate() {
      for m in pattern.find_iter(&self.j) {
        raw_matches.push(RawMatch {
          start: m.start(),
          end: m.end(),
          pattern_name: name,
          pattern,
          text: m.as_str(),
          priority,
        });
      }
    }

    raw_matches.sort_by(|a, b| {
      a.start
        .cmp(&b.start)
        .then(b.end.cmp(&a.end))
        .then(a.priority.cmp(&b.priority))
    });

    let mut last_end = 0;
    for rm in raw_matches {
      if rm.start < last_end {
        continue;
      }

      let captures: Vec<(&str, usize)> = if rm.pattern.captures_len() == 1 {
        vec![(rm.text, 0)]
      } else if let Some(captures) = rm.pattern.captures(rm.text) {
        if let Some(capture) = captures.name("match") {
          vec![(capture.as_str(), capture.start())]
        } else if captures.len() > 1 {
          captures
            .iter()
            .skip(1)
            .flatten()
            .map(|capture| (capture.as_str(), capture.start()))
            .collect()
        } else {
          vec![(rm.text, 0)]
        }
      } else {
        continue;
      };

      if rm.pattern_name != "bash" {
        for (subtext, substart) in captures.iter() {
          let subtext = if rm.pattern_name == "url" {
            trim_url_punctuation(subtext)
          } else {
            *subtext
          };
          let j_match_start = rm.start + *substart;

          if j_match_start < self.map.len() {
            let (v_line, v_char, visual_x) = self.map[j_match_start];

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
          mat.hint = Some(hint.to_string());
          previous.insert(mat.text, hint.to_string());
        }
      }
    } else {
      for mat in &mut matches {
        if let Some(hint) = hints.pop() {
          mat.hint = Some(hint.to_string());
        }
      }
    }

    if reverse {
      matches.reverse();
    }

    matches
  }
}

fn trim_url_punctuation(mut url: &str) -> &str {
  loop {
    let trimmed = url.trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '!' | '?'));
    if let Some(without_paren) = trimmed.strip_suffix(')') {
      let open = without_paren.chars().filter(|&c| c == '(').count();
      let close = without_paren.chars().filter(|&c| c == ')').count();
      if close >= open {
        url = without_paren;
        continue;
      }
    }
    if let Some(without_bracket) = trimmed.strip_suffix(']') {
      let open = without_bracket.chars().filter(|&c| c == '[').count();
      let close = without_bracket.chars().filter(|&c| c == ']').count();
      if close >= open {
        url = without_bracket;
        continue;
      }
    }
    return trimmed;
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
    assert_eq!(results.get(0).unwrap().text, "/tmp/foo/bar_lol");
    assert_eq!(results.get(1).unwrap().text, "/var/log/boot-strap.log");
    assert_eq!(results.get(2).unwrap().text, "../log/kern.log");
  }

  #[test]
  fn match_routes() {
    let lines = split("Lorem /app/routes/$routeId/$objectId, lorem\n Lorem /app/routes/$sectionId");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().text, "/app/routes/$routeId/$objectId");
    assert_eq!(results.get(1).unwrap().text, "/app/routes/$sectionId");
  }

  #[test]
  fn match_home() {
    let lines = split("Lorem ~/.gnu/.config.txt, lorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text, "~/.gnu/.config.txt");
  }

  #[test]
  fn match_slugs() {
    let lines = split("Lorem dev/api/[slug]/foo, lorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text, "dev/api/[slug]/foo");
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
    assert_eq!(results.get(0).unwrap().text, "fd70b5695");
    assert_eq!(results.get(1).unwrap().text, "5246ddf");
    assert_eq!(results.get(2).unwrap().text, "f924213");
    assert_eq!(results.get(3).unwrap().text, "973113963b491874ab2e372ee60d4b4cb75f717c");
  }

  #[test]
  fn match_ips() {
    let lines = split("Lorem ipsum 127.0.0.1 lorem\n Lorem 255.255.10.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text, "127.0.0.1");
    assert_eq!(results.get(1).unwrap().text, "255.255.10.255");
    assert_eq!(results.get(2).unwrap().text, "127.0.0.1");
  }

  #[test]
  fn match_ipv6s() {
    let lines = split("Lorem ipsum fe80::2:202:fe4 lorem\n Lorem 2001:67c:670:202:7ba8:5e41:1591:d723 lorem fe80::2:1 lorem ipsum fe80:22:312:fe::1%eth0");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text, "fe80::2:202:fe4");
    assert_eq!(results.get(1).unwrap().text, "2001:67c:670:202:7ba8:5e41:1591:d723");
    assert_eq!(results.get(2).unwrap().text, "fe80::2:1");
    assert_eq!(results.get(3).unwrap().text, "fe80:22:312:fe::1%eth0");
  }

  #[test]
  fn match_markdown_urls() {
    let lines = split("Lorem ipsum [link](https://github.io?foo=bar) ![](http://cdn.com/img.jpg) lorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().pattern, "markdown_url");
    assert_eq!(results.get(0).unwrap().text, "https://github.io?foo=bar");
    assert_eq!(results.get(1).unwrap().pattern, "markdown_url");
    assert_eq!(results.get(1).unwrap().text, "http://cdn.com/img.jpg");
  }

  #[test]
  fn match_urls() {
    let lines = split("Lorem ipsum https://www.rust-lang.org/tools. lorem\n Lorem (https://crates.io) lorem https://en.wikipedia.org/wiki/Rust_(programming_language) lorem ssh://github.io");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text, "https://www.rust-lang.org/tools");
    assert_eq!(results.get(0).unwrap().pattern, "url");
    assert_eq!(results.get(1).unwrap().text, "https://crates.io");
    assert_eq!(results.get(1).unwrap().pattern, "url");
    assert_eq!(
      results.get(2).unwrap().text,
      "https://en.wikipedia.org/wiki/Rust_(programming_language)"
    );
    assert_eq!(results.get(2).unwrap().pattern, "url");
    assert_eq!(results.get(3).unwrap().text, "ssh://github.io");
    assert_eq!(results.get(3).unwrap().pattern, "url");
  }

  #[test]
  fn match_addresses() {
    let lines = split("Lorem 0xfd70b5695 0x5246ddf lorem\n Lorem 0x973113tlorem");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text, "0xfd70b5695");
    assert_eq!(results.get(1).unwrap().text, "0x5246ddf");
    assert_eq!(results.get(2).unwrap().text, "0x973113");
  }

  #[test]
  fn match_hex_colors() {
    let lines = split("Lorem #fd7b56 lorem #FF00FF\n Lorem #00fF05 lorem #abcd00 lorem #afRR00");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text, "#fd7b56");
    assert_eq!(results.get(1).unwrap().text, "#FF00FF");
    assert_eq!(results.get(2).unwrap().text, "#00fF05");
    assert_eq!(results.get(3).unwrap().text, "#abcd00");
  }

  #[test]
  fn match_ipfs() {
    let lines = split("Lorem QmRdbNSxDJBXmssAc9fvTtux4duptMvfSGiGuq6yHAQVKQ lorem Qmfoobar");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(
      results.get(0).unwrap().text,
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
    assert_eq!(results.get(0).unwrap().text, "src/main.rs");
  }

  #[test]
  fn match_diff_b() {
    let lines = split("Lorem lorem\n+++ b/src/main.rs");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text, "src/main.rs");
  }

  #[test]
  fn match_diff_summary() {
    let lines = split("diff --git a/samples/test1 b/samples/test2");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().text, "samples/test1");
    assert_eq!(results.get(1).unwrap().text, "samples/test2");
  }
  #[test]
  fn priority() {
    let lines = split("Lorem [link](http://foo.bar) ipsum CUSTOM-52463 lorem ISSUE-123 lorem\nLorem /var/fd70b569/9999.log 52463 lorem\n Lorem 973113 lorem 123e4567-e89b-12d3-a456-426655440000 lorem 8888 lorem\n  https://crates.io/23456/fd70b569 lorem");
    let custom = ["CUSTOM-[0-9]{4,}", "ISSUE-[0-9]{3}"].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 9);
    assert_eq!(results.get(0).unwrap().text, "http://foo.bar");
    assert_eq!(results.get(1).unwrap().text, "CUSTOM-52463");
    assert_eq!(results.get(2).unwrap().text, "ISSUE-123");
    assert_eq!(results.get(3).unwrap().text, "/var/fd70b569/9999.log");
    assert_eq!(results.get(4).unwrap().text, "52463");
    assert_eq!(results.get(5).unwrap().text, "973113");
    assert_eq!(results.get(6).unwrap().text, "123e4567-e89b-12d3-a456-426655440000");
    assert_eq!(results.get(7).unwrap().text, "8888");
    assert_eq!(results.get(8).unwrap().text, "https://crates.io/23456/fd70b569");
  }

  #[test]
  fn test_visual_width() {
    assert_eq!(visual_width("hello"), 5);
    assert_eq!(visual_width("\x1b[31mhello\x1b[m"), 5);
    assert_eq!(visual_width("\x1b[1;31mhello\x1b[0m world"), 11);
    assert_eq!(visual_width("    \x1b[31mmodified:\x1b[m   "), 16);
    assert_eq!(visual_width("\t\x1b[31mmodified:\x1b[m    "), 21);
    assert_eq!(visual_width("a\t"), 8);
    // OSC 8 hyperlinks and charset designation escapes
    assert_eq!(
      visual_width(
        "\x1b]8;id=jkb4deb49e;file:///usr/local/google/home/sselcuk/.tmux.conf#L117\x1b\\~/.tmux.conf\x1b]8;;\x1b\\:"
      ),
      13
    );
    assert_eq!(visual_width("\x1b(B\x1b[mhello\x1b(0"), 5);
    // Literal \033 or \e in source code should not be stripped
    assert_eq!(visual_width(r"\033[0m"), 7);
    assert_eq!(visual_width(r"\e[31m"), 6);
    // Emoji with variation selector \u{fe0f}
    assert_eq!(visual_width("⚠️"), 2);
  }

  #[test]
  fn match_with_osc8_hyperlinks() {
    let lines = split("in your \x1b[4m\x1b]8;id=jkb4deb49e;file:///usr/local/google/home/sselcuk/.tmux.conf#L117\x1b\\~/.tmux.conf\x1b[0m\x1b]8;;\x1b\\:");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "~/.tmux.conf");
    assert_eq!(results[0].visual_x, 8);
  }

  #[test]
  fn no_false_positive_line_wrap() {
    // Two short lines where line 0 is the longest line and ends with a path, and line 1 starts at col 0
    let lines = split("check /usr/local/bin\n/var/log/syslog");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].text, "/usr/local/bin");
    assert_eq!(results[0].y, 0);
    assert_eq!(results[1].text, "/var/log/syslog");
    assert_eq!(results[1].y, 1);
  }

  #[test]
  fn test_no_line_join_across_boundaries() {
    let lines = split("\x1b[33mca2628a\x1b[39m statusbar: format GCal countdown as 'm' with red negative minutes, send 5m urgent notifications, and dedupe by time range\n\x1b[33mb828bec\x1b[39m use softlink for bg image");
    let custom = [].to_vec();
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);

    let texts: Vec<&str> = results.iter().map(|m| m.text).collect();
    assert!(texts.contains(&"ca2628a"), "ca2628a should be matched");
    assert!(texts.contains(&"b828bec"), "b828bec should be matched");
    assert!(
      !texts.contains(&"eb828bec"),
      "eb828bec should not be matched across line boundary"
    );
  }

  #[test]
  fn test_url_not_split_by_custom_path_regex() {
    let lines = split("origin    https://github.com/catgoose/nvim-colorizer.lua (fetch)");
    let custom =
      vec![r"~?/{0,3}(?:[\w.*${}:@+~%]+(?:[=\-][\w.*${}:@+~%]+)*/)+(?:[\w.*${}:@+~%]+(?:[?@=\-][\w.*${}:@+~%]*)*)?"];
    let state = State::new(&lines, "abcd", &custom);
    let results = state.matches(false, false);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "https://github.com/catgoose/nvim-colorizer.lua");
  }
}
