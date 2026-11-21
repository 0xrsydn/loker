//! For comparing reader changes: reads JSON lines `{"id", "text"}` on stdin
//! and prints every description reader's output for each text. With an
//! argument (`experience`, `visa`, `pay`, `stated`, `open_wider`), runs only
//! that reader and prints nothing, for timing.
use std::io::{BufRead, Write};

use loker_core::{eligibility_text, experience_text, pay_text, visa_text};

fn main() -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    let only = std::env::args().nth(1);
    for line in std::io::stdin().lock().lines() {
        let v: serde_json::Value = serde_json::from_str(&line?)?;
        let text = v["text"].as_str().unwrap_or("");
        if let Some(only) = only.as_deref() {
            let hit = match only {
                "experience" => experience_text::experience_from_text(text).is_some(),
                "visa" => visa_text::visa_from_text(text).is_some(),
                "pay" => pay_text::pay_from_text(text).is_some(),
                "stated" => eligibility_text::stated(text).is_some(),
                "open_wider" => eligibility_text::open_wider(text),
                // What reading a stored job does: one parse, both readers.
                "read_time" => {
                    let p = loker_core::posting::Posting::parse(text);
                    visa_text::read(&p).is_some() | experience_text::read(&p).is_some()
                }
                "read_time_separate" => {
                    visa_text::visa_from_text(text).is_some()
                        | experience_text::experience_from_text(text).is_some()
                }
                _ => anyhow::bail!("unknown reader {only}"),
            };
            std::hint::black_box(hit);
            continue;
        }
        let stated = eligibility_text::stated(text).map(|s| {
            serde_json::json!({
                "countries": s.countries, "regions": s.regions, "onlyCities": s.only_cities,
                "worldwide": s.worldwide, "tz": s.tz, "evidence": s.evidence,
            })
        });
        writeln!(
            out,
            "{}",
            serde_json::json!({
                "id": v["id"],
                "experience": experience_text::experience_from_text(text),
                "visa": visa_text::visa_from_text(text),
                "pay": pay_text::pay_from_text(text),
                "stated": stated,
                "openWider": eligibility_text::open_wider(text),
            })
        )?;
    }
    Ok(())
}
