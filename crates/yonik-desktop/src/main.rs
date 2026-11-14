#![deny(unsafe_code)]

mod dispatch_error;

// Модуль содержит только декларацию сгенерированного FFI.
#[allow(unsafe_code)]
mod bridge;

#[forbid(unsafe_code)]
mod bridge_impl;

use std::{num::NonZeroU16, os::unix::fs::DirBuilderExt, path::PathBuf};
use yonik_desktop::HostConfig;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut data_dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .ok_or("Не задан домашний каталог данных")?
        // Старый каталог сохраняет доступ к зашифрованной БД после ребрендинга.
        .join("tofu");
    let mut engine_dir = None;
    let mut listen_port = None;
    let mut explicit_data_dir = false;
    let mut smoke_test = false;
    let mut start_hidden = false;
    let mut screenshot = None;
    let mut script = None;
    let mut trace = None;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--data-dir") => {
                data_dir = PathBuf::from(arguments.next().ok_or("Не задан --data-dir")?);
                explicit_data_dir = true;
            }
            Some("--smoke-test") => smoke_test = true,
            Some("--start-hidden") => start_hidden = true,
            Some("--smoke-script") => {
                script = Some(PathBuf::from(
                    arguments.next().ok_or("Не задан --smoke-script")?,
                ));
            }
            Some("--smoke-trace") => {
                trace = Some(PathBuf::from(
                    arguments.next().ok_or("Не задан --smoke-trace")?,
                ));
            }
            Some("--smoke-screenshot") => {
                screenshot = Some(PathBuf::from(
                    arguments.next().ok_or("Не задан --smoke-screenshot")?,
                ));
            }
            Some("--engine-dir") => {
                engine_dir = Some(PathBuf::from(
                    arguments.next().ok_or("Не задан --engine-dir")?,
                ));
            }
            Some("--listen-port") => {
                let port = arguments.next().ok_or("Не задан --listen-port")?;
                listen_port = Some(
                    port.to_str()
                        .ok_or("Порт должен быть числом")?
                        .parse::<NonZeroU16>()
                        .map_err(|_| "Порт должен быть в диапазоне 1–65535")?,
                );
            }
            _ => return Err("Неизвестный параметр командной строки".into()),
        }
    }
    if (smoke_test && !explicit_data_dir)
        || ((screenshot.is_some() || script.is_some() || trace.is_some()) && !smoke_test)
    {
        return Err(
            "--smoke-test требует явный --data-dir; screenshot доступен только в smoke-test".into(),
        );
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&data_dir)?;
    let data_dir = std::fs::canonicalize(data_dir)?;
    let engine_dir = engine_dir.unwrap_or_else(|| data_dir.join("engines"));
    let mut config = HostConfig::new(data_dir.clone(), engine_dir).with_start_hidden(start_hidden);
    if let Some(port) = listen_port {
        config = config.with_listen_port(port);
    }
    if smoke_test {
        let mut key = zeroize::Zeroizing::new([0_u8; 32]);
        getrandom::getrandom(key.as_mut()).map_err(|_| "Не удалось создать тестовый ключ")?;
        config = config.with_ephemeral_key(*key);
    }
    bridge_impl::configure(config).map_err(|_| "Конфигурация уже задана")?;
    let data_dir = data_dir.to_str().ok_or("Путь данных должен быть UTF-8")?;
    let screenshot = screenshot
        .as_deref()
        .map(|path| path.to_str().ok_or("Путь screenshot должен быть UTF-8"))
        .transpose()?
        .unwrap_or("");
    let script = script
        .as_deref()
        .map(|path| path.to_str().ok_or("Путь script должен быть UTF-8"))
        .transpose()?
        .unwrap_or("");
    let trace = trace
        .as_deref()
        .map(|path| path.to_str().ok_or("Путь trace должен быть UTF-8"))
        .transpose()?
        .unwrap_or("");
    let code = bridge::ffi::run_application(
        &data_dir.into(),
        smoke_test,
        &screenshot.into(),
        &script.into(),
        &trace.into(),
    );
    if code == 0 {
        Ok(())
    } else {
        Err(format!("Qt завершился с кодом {code}").into())
    }
}
