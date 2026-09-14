use crate::Result;
use std::{fs, path::Path};

/// Windows CRT/LLVM response-file quoting. Preserve empty arguments and the
/// even/odd backslash rule before quotes (including paths ending in a slash).
pub fn parse(text: &str) -> Result<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut result = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i == chars.len() {
            break;
        }
        let mut arg = String::new();
        let mut quoted = false;
        while i < chars.len() && (quoted || !chars[i].is_whitespace()) {
            let mut slashes = 0;
            while i < chars.len() && chars[i] == '\\' {
                slashes += 1;
                i += 1;
            }
            if i < chars.len() && chars[i] == '"' {
                arg.extend(std::iter::repeat_n('\\', slashes / 2));
                if slashes % 2 == 1 {
                    arg.push('"');
                } else if quoted && i + 1 < chars.len() && chars[i + 1] == '"' {
                    arg.push('"');
                    i += 1;
                } else {
                    quoted = !quoted;
                }
                i += 1;
            } else {
                arg.extend(std::iter::repeat_n('\\', slashes));
                if i < chars.len() && (quoted || !chars[i].is_whitespace()) {
                    arg.push(chars[i]);
                    i += 1;
                }
            }
        }
        if quoted {
            return Err("unclosed quote in response file".into());
        }
        result.push(arg);
    }
    Ok(result)
}

pub fn read(path: &Path) -> Result<Vec<String>> {
    let bytes = fs::read(path)?;
    let text = if bytes.starts_with(&[0xff, 0xfe]) {
        if bytes.len() % 2 != 0 {
            return Err("invalid UTF-16 response file".into());
        }
        String::from_utf16(
            &bytes[2..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|x| u16::from_le_bytes([x[0], x[1]]))
                .collect::<Vec<_>>(),
        )?
    } else {
        String::from_utf8(bytes)?
            .trim_start_matches('\u{feff}')
            .to_owned()
    };
    parse(&text)
}

pub fn quote(arg: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for ch in arg.chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        result.extend(std::iter::repeat_n(
            '\\',
            if ch == '"' { slashes * 2 + 1 } else { slashes },
        ));
        slashes = 0;
        result.push(ch);
    }
    result.extend(std::iter::repeat_n('\\', slashes * 2));
    result.push('"');
    result
}

pub fn expand(arguments: &[String], cwd: &Path, log: &Path) -> Result<Vec<String>> {
    fn visit(
        args: &[String],
        cwd: &Path,
        log: &Path,
        count: &mut usize,
        depth: usize,
    ) -> Result<Vec<String>> {
        if depth > 16 {
            return Err("response file recursion limit exceeded".into());
        }
        let mut result = Vec::new();
        for arg in args {
            if let Some(name) = arg.strip_prefix('@') {
                let path = cwd.join(name);
                fs::copy(&path, log.join(format!("original-{}.rsp", *count)))?;
                *count += 1;
                result.extend(visit(&read(&path)?, cwd, log, count, depth + 1)?);
            } else {
                result.push(arg.clone());
            }
        }
        Ok(result)
    }
    visit(arguments, cwd, log, &mut 0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_arguments_round_trip() {
        let args = [
            "",
            "plain",
            "C:\\Program Files\\",
            "C:\\a b\\file.obj",
            "quote\"x",
            "a\\\"b",
            "åäö 🦀",
            "tab\tvalue",
        ];
        let command = args.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ");
        assert_eq!(parse(&command).unwrap(), args);
    }
    #[test]
    fn response_lines_and_unclosed_quotes() {
        assert_eq!(
            parse("/OUT:\"a b.exe\"\r\n\"input.obj\"\n").unwrap(),
            ["/OUT:a b.exe", "input.obj"]
        );
        assert!(parse("\"broken").is_err());
    }
}
