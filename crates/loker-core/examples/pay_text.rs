//! For eval/pay: reads JSON lines `{"id", "text"}` on stdin and prints
//! `{"id", "pay"}`, the pay `pay_text::pay_from_text` reads from each text.
use std::io::{BufRead, Write};

fn main() -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let v: serde_json::Value = serde_json::from_str(&line?)?;
        let pay = loker_core::pay_text::pay_from_text(v["text"].as_str().unwrap_or(""));
        writeln!(out, "{}", serde_json::json!({ "id": v["id"], "pay": pay }))?;
    }
    Ok(())
}
