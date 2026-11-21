use std::io::{BufRead, Write};

fn main() -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let v: serde_json::Value = serde_json::from_str(&line?)?;
        let visa = loker_core::visa_text::visa_from_text(v["text"].as_str().unwrap_or(""));
        writeln!(
            out,
            "{}",
            serde_json::json!({ "id": v["id"], "visa": visa })
        )?;
    }
    Ok(())
}
