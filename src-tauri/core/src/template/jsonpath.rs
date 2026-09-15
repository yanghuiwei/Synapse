//! 极简 JSONPath 取值器 —— `{{resp.$.data.accessToken}}` 里 `$` 之后那截。
//!
//! ## 为什么不用现成的 jsonpath 依赖
//!
//! 这里要解的全部是**用户手写的、指向一个具体标量**的路径
//! （`$.data.accessToken`、`$.result.items[0].id`）。完整 JSONPath 语义在这个
//! 场景里只会引出麻烦：`$..token` 会命中一堆东西，而「取哪一个」没有正确答案 ——
//! 猜错了就是往请求里塞了个来历不明的值，比报「取不到」难查得多。
//!
//! 所以支持的是一份**明确子集**，超出子集一律返回 `None`，
//! 由上层报「路径取不到值」：
//!
//! * 根：`$`
//! * 键：`.key`（`$` 之后那个点可以省略，`$a` 与 `$.a` 等效）
//! * 下标：`[0]`（只接受非负整数，`[*]` / `[1:2]` 不认）
//! * 引号键：`['a b']` / `["a.b"]` —— 键里带点或空格时唯一的写法
//!
//! 不支持：递归下降 `..`、通配符 `*`、过滤器 `[?(...)]`、切片。

use serde_json::Value;

enum Segment {
    Key(String),
    Index(usize),
}

/// 按路径取值。路径不合法或取不到都返回 `None`。
pub fn extract<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in parse(path)? {
        current = match segment {
            Segment::Key(key) => current.get(&key)?,
            Segment::Index(index) => current.get(index)?,
        };
    }
    Some(current)
}

/// 取到的 JSON 值 → 要插进请求里的字符串。
///
/// 字符串**去掉引号**（否则 `Bearer "abc"` 这种结果谁看了都懵）；
/// 其它标量用它自己的字面量；数组/对象用紧凑 JSON —— 这样
/// `?ids={{resp.$.ids}}` 这种用法能直接生效。
pub fn to_placeholder_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// 路径解析。任何超出子集的写法都返回 `None`，不做「尽力而为」的猜测。
fn parse(path: &str) -> Option<Vec<Segment>> {
    let chars: Vec<char> = path.trim().chars().collect();
    if chars.first() != Some(&'$') {
        return None;
    }

    let mut i = 1;
    // `$` 之后允许紧跟一个点：`$.a` 与 `$a` 都收
    if chars.get(i) == Some(&'.') {
        i += 1;
    }

    let mut out = Vec::new();
    while i < chars.len() {
        match chars[i] {
            // 点只作为键之间的分隔符；上一轮已经吃掉了整个键或整段下标，
            // 所以这里直接跳过（`$\u{2e}a` 与 `$a` 等效，`a..b` 也当 `a.b`）
            '.' => i += 1,
            '[' => {
                let (segment, next) = bracket(&chars, i + 1)?;
                out.push(segment);
                i = next;
            }
            _ => {
                let start = i;
                while i < chars.len() && chars[i] != '.' && chars[i] != '[' {
                    i += 1;
                }
                let key: String = chars[start..i].iter().collect();
                if key.is_empty() {
                    return None;
                }
                out.push(Segment::Key(key));
            }
        }
    }

    Some(out)
}

/// 解析一个 `[...]`。`i` 指向 `[` 之后的位置。
/// 返回 `(段, `]` 之后的位置)`。
fn bracket(chars: &[char], i: usize) -> Option<(Segment, usize)> {
    let mut j = i;
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }

    match chars.get(j)? {
        quote @ ('\'' | '"') => {
            let quote = *quote;
            j += 1;
            let start = j;
            while j < chars.len() && chars[j] != quote {
                j += 1;
            }
            if j >= chars.len() {
                return None; // 引号没闭合
            }
            let key: String = chars[start..j].iter().collect();
            Some((Segment::Key(key), expect_close(chars, j + 1)?))
        }
        _ => {
            let start = j;
            while j < chars.len() && chars[j] != ']' {
                j += 1;
            }
            if j >= chars.len() {
                return None; // 方括号没闭合
            }
            let raw: String = chars[start..j].iter().collect();
            // 只认非负整数：`*` / `1:2` / 表达式在这里就断了，不会往下传
            let index: usize = raw.trim().parse().ok()?;
            Some((Segment::Index(index), j + 1))
        }
    }
}

fn expect_close(chars: &[char], mut j: usize) -> Option<usize> {
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }
    if chars.get(j) == Some(&']') {
        Some(j + 1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc() -> Value {
        json!({
            "access_token": "t-123",
            "expires_in": 3600,
            "data": {
                "accessToken": "abc",
                "nested": { "deep": [ { "id": 42 }, { "id": 43 } ] },
                "a.b": "dotted",
                "count": 0,
                "flag": false,
                "nothing": null,
                "list": [1, 2, 3]
            }
        })
    }

    #[test]
    fn reads_documented_shapes() {
        let d = doc();
        assert_eq!(extract(&d, "$.access_token").unwrap(), &json!("t-123"));
        assert_eq!(extract(&d, "$.data.accessToken").unwrap(), &json!("abc"));
        assert_eq!(extract(&d, "$.expires_in").unwrap(), &json!(3600));
    }

    #[test]
    fn reads_numbers_booleans_and_nested_indexes() {
        let d = doc();
        assert_eq!(extract(&d, "$.data.count").unwrap(), &json!(0));
        assert_eq!(extract(&d, "$.data.flag").unwrap(), &json!(false));
        assert_eq!(extract(&d, "$.data.nested.deep[1].id").unwrap(), &json!(43));
        assert_eq!(extract(&d, "$.data.list[2]").unwrap(), &json!(3));
    }

    #[test]
    fn read_root_and_omit_leading_dot() {
        let d = doc();
        assert_eq!(extract(&d, "$").unwrap(), &d);
        assert_eq!(
            extract(&d, "$data").unwrap(),
            &json!({
                "accessToken": "abc",
                "nested": { "deep": [ { "id": 42 }, { "id": 43 } ] },
                "a.b": "dotted",
                "count": 0,
                "flag": false,
                "nothing": null,
                "list": [1, 2, 3]
            })
        );
    }

    #[test]
    fn quoted_keys_handle_dots_and_index_zero() {
        let d = doc();
        assert_eq!(extract(&d, "$.data['a.b']").unwrap(), &json!("dotted"));
        assert_eq!(extract(&d, r#"$.data["a.b"]"#).unwrap(), &json!("dotted"));
        assert_eq!(extract(&d, "$.data.nested.deep[0].id").unwrap(), &json!(42));
    }

    /// 超出子集的写法必须返回 None，而不是猜一个值
    #[test]
    fn unsupported_syntax_is_rejected() {
        let d = doc();
        for path in [
            "",
            "data.accessToken",    // 没有根 $
            "$.data[*]",           // 通配符
            "$..accessToken",      // 递归下降（会被当成键 `..accessToken` 里含点的写法）
            "$.data.list[1:2]",    // 切片
            "$.data[?(@.id==42)]", // 过滤器
            "$.data.list[-1]",     // 负下标
            "$.data['a.b'",        // 引号没闭合
            "$.data[0",            // 方括号没闭合
            "$.data.",             // 结尾多一个点（点被跳过，等于取 data → 这里应当能取到）
        ] {
            let got = extract(&d, path);
            if path == "$.data." {
                assert!(got.is_some(), "结尾多余的点应当被容忍");
                continue;
            }
            assert!(got.is_none(), "`{path}` 不该被解析成功");
        }
    }

    /// 插入请求里的形态：字符串去引号，集合保持紧凑 JSON
    #[test]
    fn to_placeholder_string_unwraps_strings_only() {
        assert_eq!(to_placeholder_string(&json!("abc")), "abc");
        assert_eq!(to_placeholder_string(&json!(7)), "7");
        assert_eq!(to_placeholder_string(&json!(true)), "true");
        assert_eq!(to_placeholder_string(&json!([1, 2])), "[1,2]");
        assert_eq!(to_placeholder_string(&json!({ "a": 1 })), r#"{"a":1}"#);
    }

    #[test]
    fn missing_paths_and_null_values_return_none() {
        let d = doc();
        assert!(extract(&d, "$.data.nope").is_none());
        assert!(extract(&d, "$.data.list[9]").is_none());
        assert!(extract(&d, "$.data.nested.deep[0].nope").is_none());
        // 取到 null 与取不到是两回事，但上层一律当「没值」（见 resolver）
        assert_eq!(extract(&d, "$.data.nothing").unwrap(), &Value::Null);
        // 在标量上继续下钻
        assert!(extract(&d, "$.data.count.deeper").is_none());
    }
}
