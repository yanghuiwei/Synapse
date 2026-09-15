//! shell 词法分析：把「粘进来的一段 curl 命令」切成词。
//!
//! ## 为什么单独一个模块、而且 TDD 先行
//!
//! 这里全部是**引号与转义的边界情况**，而且外面套了两种 shell 方言
//! （bash 的 `\`、CMD 的 `^`、PowerShell 的反引号）。这类代码的特点是：
//! 写的时候觉得自己想全了，实际上是靠一个个真实样本试出来的。
//!
//! 所以顺序是「先攒样本 → 再写实现」：样本全部来自
//! Chrome / Edge / Firefox 的「Copy as cURL (bash / cmd / powershell)」，
//! 以及真实用户手工拼出来的命令（`$'...'`、Windows 路径、多行 body）。
//!
//! ## 输出的形状
//!
//! [`tokenize_commands`] 直接返回**若干条命令**、每条是若干 [`Token`]：
//! 剪贴板里通常只有一条，但「可粘多条」（`import.curlPlaceholder`）要求一次能贴好几条，
//! 而切分点又必须**引号感知**（`-H 'x-tool: curl'` 里的 `curl` 不是新命令）——
//! 干脆一个状态机里一次做完，免得引号逻辑写两遍、两边迟早分叉。
//!
//! ## 不做的两件事（有意）
//!
//! * **不做变量展开**：`$HOME` 原样保留。展开成导入那台机器的环境变量是错的，
//!   而且会把 `{{env.x}}` 这类占位符语义搅混。
//! * **不解释 curl 参数**：`-H` 后面跟什么、哪个参数吃值，是 [`super::curl`] 的事。
//!   这里只负责「词」，越界了就没法单独测。

use serde::Serialize;

/// 一个 shell 词。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Token {
    /// 解完转义之后的文本
    pub text: String,
    /// 这个词有没有被引号包起来。
    ///
    /// 存在的理由是 `-d ''`（显式给了个空 body）与 `-d`（漏了值）必须是两种结果 ——
    /// 只看 `text` 都是空串，分不出来。
    pub quoted: bool,
}

impl Token {
    fn bare(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            quoted: false,
        }
    }

    fn quoted(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            quoted: true,
        }
    }

    /// 是不是 `curl` 这个可执行文件（`curl` / `curl.exe` / `/usr/bin/curl` 都算）
    pub fn is_curl_invocation(&self) -> bool {
        let name = self
            .text
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&self.text)
            .to_ascii_lowercase();
        name == "curl" || name == "curl.exe"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LexError {
    /// 引号没闭合
    UnterminatedQuote { quote: char },
    /// 命令以转义符 / 续行符结尾（粘贴被截断了）
    TrailingEscape { ch: char },
}

impl LexError {
    pub fn code(&self) -> &'static str {
        crate::error::codes::IMPORT_PARSE
    }

    pub fn debug(&self) -> String {
        match self {
            LexError::UnterminatedQuote { quote } => format!("unterminated quote `{quote}`"),
            LexError::TrailingEscape { ch } => format!("trailing escape `{ch}` at end of input"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// 引号外
    Bare,
    /// `'...'`：里面**什么都不是特殊字符**，连 `\` 也是字面量
    Single,
    /// `"..."`：只有 `\` 后跟 `" $ \` \\ 换行` 才是转义，其他 `\x` 保留反斜杠（bash 的真实行为）
    Double,
    /// `$'...'`：ANSI-C 转义
    Ansi,
}

/// 把整段文本切成「若干条命令 × 若干词」。
///
/// 命令边界：**行首**出现 `curl` / `curl.exe` / `/path/to/curl`（不区分大小写）。
/// 行首 = 这一行在它之前只有空白；续行符（`\` / `^` / `` ` `` 加换行）不算换行。
pub fn tokenize_commands(input: &str) -> Result<Vec<Vec<Token>>, LexError> {
    // CRLF 先归一：留着 `\r` 会让「行尾是不是续行符」的判断全部错位
    let normalized = input.replace("\r\n", "\n");
    let chars: Vec<char> = normalized.chars().collect();

    let mut commands: Vec<Vec<Token>> = Vec::new();
    let mut current: Vec<Token> = Vec::new();

    let mut word = String::new();
    let mut word_started = false;
    let mut word_quoted = false;
    // 这个词是不是从行首开始的 —— 只有行首的 `curl` 才算新命令
    let mut word_at_line_start = false;
    let mut line_start = true;

    let mut state = State::Bare;
    let mut quote_char = '\'';

    let mut i = 0;

    // 收尾：把正在攒的词推进当前命令。
    //
    // 刻意**不在这里**重置 `word_started` / `word_quoted`：
    // 重置放在唯一的调用点（空白分支）之后，最后一次 flush 之后就不会有
    // 「赋了没人读」的赋值（宏展开出来的这种警告是真的，不是假警报）
    macro_rules! flush_word {
        () => {
            if word_started {
                let token = if word_quoted {
                    Token::quoted(std::mem::take(&mut word))
                } else {
                    Token::bare(std::mem::take(&mut word))
                };
                // 行首的 curl 开一条新命令（`-H 'x-tool: curl'` 不在行首，因此不会误切）
                if word_at_line_start && token.is_curl_invocation() && !current.is_empty() {
                    commands.push(std::mem::take(&mut current));
                }
                current.push(token);
            }
        };
    }

    while i < chars.len() {
        let c = chars[i];

        match state {
            State::Bare => match c {
                // ---- 续行：`\` / `^` / ` 加换行 → 整段吃掉，不断行也不断词 ----
                '\\' | '^' | '`' if next_is_newline(&chars, i) => {
                    i += if i + 1 < chars.len() && chars[i + 1] == '\r' {
                        3
                    } else {
                        2
                    };
                    continue;
                }
                // ---- 转义：bash 的 `\x` 与 PowerShell 的 `` `x `` 都是「吃掉转义符」----
                '\\' | '`' => {
                    if i + 1 >= chars.len() {
                        return Err(LexError::TrailingEscape { ch: c });
                    }
                    word_started = true;
                    word.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                // ---- CMD 的 `^x`：吃掉 `^`，**x 重新按正常字符处理** ----
                // 关键点：`^"` 之后的引号必须还能开启字符串，
                // 否则 CMD 格式（`curl ^"https://x^"`）整条都解不出来
                '^' => {
                    i += 1;
                    continue;
                }
                '\'' => {
                    // `$'...'` 的 `$` 已经在上一步消费掉了
                    word_started = true;
                    word_quoted = true;
                    quote_char = '\'';
                    state = State::Single;
                    i += 1;
                    continue;
                }
                '"' => {
                    word_started = true;
                    word_quoted = true;
                    quote_char = '"';
                    state = State::Double;
                    i += 1;
                    continue;
                }
                '$' if matches!(chars.get(i + 1), Some('\'')) => {
                    word_started = true;
                    word_quoted = true;
                    quote_char = '\'';
                    state = State::Ansi;
                    i += 2;
                    continue;
                }
                '$' if matches!(chars.get(i + 1), Some('"')) => {
                    // bash 的 `$"..."`（locale 翻译）。当成普通双引号即可 —— 我们不做翻译
                    word_started = true;
                    word_quoted = true;
                    quote_char = '"';
                    state = State::Double;
                    i += 2;
                    continue;
                }
                c if c.is_whitespace() => {
                    flush_word!();
                    word_started = false;
                    word_quoted = false;
                    if c == '\n' {
                        line_start = true;
                    }
                    i += 1;
                    continue;
                }
                _ => {
                    if !word_started {
                        word_at_line_start = line_start;
                    }
                    word_started = true;
                    line_start = false;
                    word.push(c);
                    i += 1;
                    continue;
                }
            },

            State::Single => {
                if c == '\'' {
                    state = State::Bare;
                    i += 1;
                } else {
                    // 单引号里 `\` 就是反斜杠本身（Windows 路径全靠这条才不会被打碎）
                    word.push(c);
                    i += 1;
                }
                continue;
            }

            State::Double => {
                if c == '"' {
                    state = State::Bare;
                    i += 1;
                    continue;
                }
                if c == '\\' {
                    match chars.get(i + 1) {
                        // 反斜杠 + 换行：续行（双引号里的续行 bash 同样会吃掉）
                        Some('\n') => {
                            i += 2;
                            continue;
                        }
                        // 只有这几个字符前，`\` 才是转义符；其余保留反斜杠
                        Some(&e @ ('"' | '\\' | '$' | '`')) => {
                            word.push(e);
                            i += 2;
                            continue;
                        }
                        None => return Err(LexError::TrailingEscape { ch: '\\' }),
                        // `\n` 这种在 bash 的双引号里就是**两个字符**，不是换行
                        _ => {
                            word.push('\\');
                            i += 1;
                            continue;
                        }
                    }
                }
                // 引号内的换行是字面换行（多行 JSON body 常见）
                if c == '^' {
                    match chars.get(i + 1) {
                        Some('\n') => {
                            i += 2;
                            continue;
                        }
                        // CMD 会把 `^"` 变成一个字面引号 —— 对 curl 来说就是**收尾引号**。
                        // 于是 `^"url^"` / `^"url"` / `"url^"` 三种写法都能收尾
                        Some('"') => {
                            state = State::Bare;
                            i += 2;
                            continue;
                        }
                        // 引号里的 `^` 在 CMD 里本来就是字面量
                        _ => {
                            word.push('^');
                            i += 1;
                            continue;
                        }
                    }
                }
                word.push(c);
                i += 1;
                continue;
            }

            State::Ansi => {
                if c == '\'' {
                    state = State::Bare;
                    i += 1;
                    continue;
                }
                if c == '\\' {
                    let Some(&next) = chars.get(i + 1) else {
                        return Err(LexError::TrailingEscape { ch: '\\' });
                    };
                    if next == '\n' {
                        i += 2;
                        continue;
                    }
                    let (text, consumed) = ansi_escape(&chars, i);
                    word.push_str(&text);
                    i += consumed;
                    continue;
                }
                word.push(c);
                i += 1;
                continue;
            }
        }
    }

    if state != State::Bare {
        return Err(LexError::UnterminatedQuote { quote: quote_char });
    }
    flush_word!();
    if !current.is_empty() {
        commands.push(current);
    }
    // 全是空白的输入切成 0 条命令，调用方据此报「没识别出接口」
    Ok(commands)
}

fn next_is_newline(chars: &[char], i: usize) -> bool {
    match chars.get(i + 1) {
        Some('\n') => true,
        // 归一化之后理论上不会走到这里，留着是为了万一有孤立的 `\r`
        Some('\r') => matches!(chars.get(i + 2), Some('\n')),
        _ => false,
    }
}

/// 解一个 ANSI-C 转义序列（`$'...'` 专用），返回（替换文本，消费的字符数）。
///
/// 认不出来的序列按 bash 的做法**退化成那个字符本身**，不报错 ——
/// 剪贴板里混进一个我们不认识的反斜杠，不该让整条命令导不进来。
fn ansi_escape(chars: &[char], start: usize) -> (String, usize) {
    // start 指向 `\`
    let Some(&c) = chars.get(start + 1) else {
        return (String::from("\\"), 1);
    };
    let simple = match c {
        'n' => Some('\n'),
        'r' => Some('\r'),
        't' => Some('\t'),
        'a' => Some('\u{7}'),
        'b' => Some('\u{8}'),
        'f' => Some('\u{c}'),
        'v' => Some('\u{b}'),
        'e' | 'E' => Some('\u{1b}'),
        '\\' => Some('\\'),
        '\'' => Some('\''),
        '"' => Some('"'),
        '?' => Some('?'),
        _ => None,
    };
    if let Some(ch) = simple {
        return (ch.to_string(), 2);
    }

    // `\xHH` / `\uHHHH` / `\UHHHHHHHH`
    let (width, radix) = match c {
        'x' => (2, 16),
        'u' => (4, 16),
        'U' => (8, 16),
        '0'..='7' => (3, 8),
        _ => return (c.to_string(), 2),
    };

    let skip = if c.is_ascii_digit() { 1 } else { 2 };
    let mut digits = String::new();
    for k in start + skip..start + skip + width {
        match chars.get(k) {
            Some(&d) if d.is_digit(radix) => digits.push(d),
            _ => break,
        }
    }
    if digits.is_empty() {
        return (c.to_string(), 2);
    }
    match u32::from_str_radix(&digits, radix)
        .ok()
        .and_then(char::from_u32)
    {
        Some(ch) => (ch.to_string(), skip + digits.len()),
        None => (c.to_string(), 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只关心「一条命令的词」，省得每个用例都写两层下标
    fn words(input: &str) -> Vec<String> {
        let commands = tokenize_commands(input).expect("应该能解出来");
        assert_eq!(commands.len(), 1, "这个用例只该切出一条命令");
        commands[0].iter().map(|t| t.text.clone()).collect()
    }

    // ============================================================
    // 真实样本：Chrome / Edge / Firefox 的「Copy as cURL」
    // ============================================================

    /// Chrome「Copy as cURL (bash)」的标准形状：单引号 + 反斜杠续行
    #[test]
    fn chrome_bash_copy() {
        let input = "curl 'https://api.example.com/v1/users?page=1&size=20' \\\n  \
             -H 'accept: application/json' \\\n  \
             -H 'authorization: Bearer eyJhbGciOiJIUzI1NiJ9.abc' \\\n  \
             -H 'content-type: application/json' \\\n  \
             --data-raw '{\"name\":\"张三\",\"tags\":[\"a\",\"b\"]}'";

        assert_eq!(
            words(input),
            vec![
                "curl",
                "https://api.example.com/v1/users?page=1&size=20",
                "-H",
                "accept: application/json",
                "-H",
                "authorization: Bearer eyJhbGciOiJIUzI1NiJ9.abc",
                "-H",
                "content-type: application/json",
                "--data-raw",
                "{\"name\":\"张三\",\"tags\":[\"a\",\"b\"]}",
            ]
        );
    }

    /// Firefox 会给一堆 `--compressed`、`-b`，并且 URL 本身不加引号
    #[test]
    fn firefox_bash_copy() {
        let input = "curl --compressed 'https://httpbin.org/post' -X POST \\\n  \
             -H 'User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64)' \\\n  \
             -H 'Cookie: sid=abc123; theme=dark' \\\n  \
             --data-raw 'x=1&y=2'";

        let w = words(input);
        assert_eq!(w[0], "curl");
        assert_eq!(w[1], "--compressed");
        assert_eq!(w[2], "https://httpbin.org/post");
        assert!(w.contains(&"POST".to_string()));
        assert!(w.contains(&"Cookie: sid=abc123; theme=dark".to_string()));
    }

    /// CMD 格式：`^` 是续行符，`\"` 是内部引号。
    /// 这是 Chrome 的「Copy as cURL (cmd)」真实形状
    #[test]
    fn cmd_copy_uses_caret_continuation() {
        let input = "curl \"https://api.example.com/v1/users\" ^\n  \
             -H \"accept: application/json\" ^\n  \
             --data-raw \"{\\\"a\\\":1}\" ^\n  \
             --compressed";
        assert_eq!(
            words(input),
            vec![
                "curl",
                "https://api.example.com/v1/users",
                "-H",
                "accept: application/json",
                "--data-raw",
                "{\"a\":1}",
                "--compressed",
            ]
        );
    }

    /// 有些工具会把引号也写成转义形式（`^"`）。
    /// 对 curl 来说 `^"` 就是一个引号，所以它既能开也能收
    #[test]
    fn caret_escaped_quotes_can_open_and_close() {
        let input = "curl ^\"https://api.example.com/v1/users^\" ^\n  \
             -H ^\"accept: application/json^\" ^\n  \
             --data-raw ^\"{\\\"a\\\":1}^\"";
        assert_eq!(
            words(input),
            vec![
                "curl",
                "https://api.example.com/v1/users",
                "-H",
                "accept: application/json",
                "--data-raw",
                "{\"a\":1}",
            ]
        );
        // 混着写也要收得住：`^"` 开、普通 `"` 收
        assert_eq!(
            words("curl ^\"https://x\" -H \"a: b\""),
            vec!["curl", "https://x", "-H", "a: b"]
        );
    }

    /// PowerShell 格式：反引号续行
    #[test]
    fn powershell_copy_uses_backtick() {
        let input = "curl.exe \"https://api.example.com/v1/users\" `\n  \
             -H \"accept: application/json\"";
        assert_eq!(
            words(input),
            vec![
                "curl.exe",
                "https://api.example.com/v1/users",
                "-H",
                "accept: application/json",
            ]
        );
    }

    // ============================================================
    // 引号与转义
    // ============================================================

    /// 单引号里反斜杠是字面量 —— Windows 路径全靠这条
    #[test]
    fn single_quotes_keep_backslashes() {
        assert_eq!(
            words(r"curl 'https://x' -H 'x-path: C:\temp\a.txt'"),
            vec!["curl", "https://x", "-H", r"x-path: C:\temp\a.txt"]
        );
    }

    /// 双引号里 `\"` 是转义，`\n` 不是（bash 的真实行为，容易想反）
    #[test]
    fn double_quotes_only_escape_a_few_chars() {
        let w = words(r#"curl "https://x" -d "{\"a\":1}" -d "a\nb""#);
        assert_eq!(w[3], r#"{"a":1}"#);
        assert_eq!(w[5], r"a\nb");
    }

    /// `$'...'`：ANSI-C 转义，`\n` 真的是换行
    #[test]
    fn ansi_c_quotes_expand_escapes() {
        let w = words(r"curl $'https://x/a\tb' -H $'x-tab:\ty' -d $'\x41\u4e2d\n'");
        assert_eq!(w[1], "https://x/a\tb");
        assert_eq!(w[3], "x-tab:\ty");
        assert_eq!(w[5], "A中\n");
    }

    /// 认不出来的 ANSI-C 序列按 bash 的做法退化成那个字符，不报错
    #[test]
    fn unknown_ansi_escape_degrades_to_the_char() {
        assert_eq!(words(r"curl $'a\qb'")[1], "aqb");
        assert_eq!(words(r"curl $'\xZZ'")[1], "xZZ");
    }

    /// 引号内换行是字面换行（多行 JSON body）
    #[test]
    fn newline_inside_quotes_is_literal() {
        let w = words("curl 'https://x' --data-raw '{\n  \"a\": 1\n}'");
        assert_eq!(w[3], "{\n  \"a\": 1\n}");
    }

    /// 双引号内的续行会被吃掉，不留下换行
    #[test]
    fn continuation_inside_double_quotes() {
        assert_eq!(words("curl 'https://x' -d \"abc\\\ndef\"")[3], "abcdef");
    }

    /// 显式空值 vs 缺值：`quoted` 必须分得出来
    #[test]
    fn empty_quoted_value_is_distinguishable() {
        let commands = tokenize_commands("curl 'https://x' -d ''").unwrap();
        let tokens = &commands[0];
        assert_eq!(tokens.len(), 4, "curl / url / -d / 空值");
        assert_eq!(tokens[3].text, "");
        assert!(tokens[3].quoted, "`-d ''` 是显式给了个空值");
        assert_eq!(tokens[2].text, "-d");
        assert!(!tokens[2].quoted);
    }

    /// CRLF：每个 `\r` 都必须被吃掉，否则「行尾是不是续行符」全部错位
    #[test]
    fn crlf_line_endings() {
        let input = "curl 'https://x' \\\r\n  -H 'accept: */*'\r\n";
        assert_eq!(words(input), vec!["curl", "https://x", "-H", "accept: */*"]);
    }

    /// 引号里的转义符不能把「下一行以 curl 开头」误判成新命令
    #[test]
    fn unterminated_quote_is_an_error() {
        assert_eq!(
            tokenize_commands("curl 'https://x"),
            Err(LexError::UnterminatedQuote { quote: '\'' })
        );
        assert_eq!(
            tokenize_commands("curl \"https://x"),
            Err(LexError::UnterminatedQuote { quote: '"' })
        );
    }

    #[test]
    fn trailing_escape_is_an_error() {
        assert_eq!(
            tokenize_commands("curl 'https://x' \\"),
            Err(LexError::TrailingEscape { ch: '\\' })
        );
    }

    // ============================================================
    // 多命令切分
    // ============================================================

    /// 剪贴板里一次粘两条
    #[test]
    fn two_commands_pasted_at_once() {
        let input = "curl 'https://a.example.com/1' -H 'x: 1'\n\
                     curl 'https://b.example.com/2' -H 'x: 2'\n";
        let commands = tokenize_commands(input).unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0][1].text, "https://a.example.com/1");
        assert_eq!(commands[1][1].text, "https://b.example.com/2");
    }

    /// 值里出现 `curl`（不在行首）**不能**切分
    #[test]
    fn curl_inside_a_value_does_not_split() {
        let input = "curl 'https://x' -H 'x-tool: curl' -H 'x-other: curl.exe'\n";
        assert_eq!(tokenize_commands(input).unwrap().len(), 1);
    }

    /// 行首的绝对路径 curl 也算一条新命令（`/usr/bin/curl` 这种）
    #[test]
    fn absolute_path_curl_starts_a_new_command() {
        let input = "curl 'https://a' -s\n/usr/bin/curl 'https://b' -s\n";
        let commands = tokenize_commands(input).unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[1][0].text, "/usr/bin/curl");
        assert!(commands[1][0].is_curl_invocation());
    }

    /// 全是空白 → 0 条命令（由调用方翻译成 E_IMPORT_DETECT）
    #[test]
    fn blank_input_has_no_commands() {
        assert!(tokenize_commands("   \n\t \n").unwrap().is_empty());
        assert!(tokenize_commands("").unwrap().is_empty());
    }
}
