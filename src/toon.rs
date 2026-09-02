//! Minimal TOON renderer (matching the AXI family). Default output is TOON;
//! `--json` is handled by the caller with serde_json Values.

fn cell(v: &str) -> String {
    if v.contains(',') || v.contains('"') || v.contains('\n') {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

pub fn header(bin: &str, description: &str) -> String {
    format!("bin: {}\ndescription: {}", bin, description)
}

pub fn list(label: &str, schema: &[&str], rows: &[Vec<String>]) -> String {
    let mut out = format!("{}[{}]{{{}}}:", label, rows.len(), schema.join(","));
    for row in rows {
        out.push('\n');
        out.push_str("  ");
        out.push_str(&row.iter().map(|c| cell(c)).collect::<Vec<_>>().join(","));
    }
    out
}

pub fn help(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = format!("help[{}]:", lines.len());
    for l in lines {
        out.push('\n');
        out.push_str(&format!("  - {}", l));
    }
    out
}

pub fn error(message: &str, code: &str, suggestions: &[String]) -> String {
    let mut out = format!("error: {}\ncode: {}", message, code);
    if !suggestions.is_empty() {
        out.push('\n');
        out.push_str(&help(suggestions));
    }
    out
}

pub fn join(blocks: &[String]) -> String {
    blocks
        .iter()
        .filter(|b| !b.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n\n")
}
