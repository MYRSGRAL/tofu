//! Однократный probe настоящего API; supervisor ограничивает ресурсы процесса.
use std::io::{self, Read};
use yonik_import::{MAX_INPUT_BYTES, import_bytes};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Vec::new();
    io::stdin()
        .take((MAX_INPUT_BYTES + 2) as u64)
        .read_to_end(&mut input)?;
    let result = import_bytes(input.clone());
    let debug = format!("{result:?}");
    assert!(!debug.contains("CANARY"), "Утечка синтетического секрета");
    match result {
        Ok(report) => {
            assert_eq!(report.original().expose().as_bytes(), input);
            let reviewed = report
                .servers
                .iter()
                .filter(|s| !s.review_flags().is_empty())
                .count();
            println!(
                "{} {} {} {}",
                report.servers.len(),
                report.issues.len(),
                reviewed,
                usize::from(report.native_json.is_some())
            );
        }
        Err(error) => {
            assert_eq!(error.input(), input);
            assert!(error.input_is_complete());
            println!("reject {:?}", error.kind());
        }
    }
    Ok(())
}
