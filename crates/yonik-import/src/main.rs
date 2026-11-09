use std::{
    collections::BTreeMap,
    fmt,
    io::{self, Write},
};

enum CliError {
    Usage,
    Import(yonik_import::ImportFailure),
    Output,
}

impl fmt::Debug for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage => formatter
                .write_str("Использование: yonik-import < источник; аргументы не принимаются"),
            Self::Import(error) => write!(formatter, "Импорт отклонён: {error}"),
            Self::Output => formatter.write_str("Не удалось записать отчёт"),
        }
    }
}

fn main() -> Result<(), CliError> {
    if std::env::args_os().len() != 1 {
        return Err(CliError::Usage);
    }
    let report = yonik_import::import_reader(io::stdin().lock()).map_err(CliError::Import)?;
    let mut protocols = BTreeMap::new();
    let mut review = BTreeMap::new();
    for server in &report.servers {
        *protocols
            .entry(server.protocol().as_str())
            .or_insert(0usize) += 1;
        for flag in server.review_flags() {
            *review.entry(format!("{flag:?}")).or_insert(0usize) += 1;
        }
    }
    let issues: Vec<_> = report.issues.iter().map(|issue| serde_json::json!({"line": issue.line, "code": format!("{:?}", issue.code), "field": format!("{:?}", issue.field)})).collect();
    let summary = serde_json::json!({"parsed": report.servers.len(), "rejected": report.issues.len(), "protocols": protocols, "review": review, "native_json": report.native_json.is_some(), "base64_envelope": report.base64_envelope, "activation": "not_checked", "issues": issues});
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, &summary).map_err(|_| CliError::Output)?;
    writeln!(output).map_err(|_| CliError::Output)
}
