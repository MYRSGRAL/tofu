//! Закрытая grammar pinned runtime; только типизированная проекция без raw payload.
//! Временный scratch не сериализуется; LocalSocks привязывает immutable scope.

use super::{EventCode, EventLog, EventSource};

pub(super) const MAX_LINE: usize = 4096;
const MAX_TURN: usize = 16 * 1024;

#[derive(Clone, Copy)]
pub(super) enum Core {
    SingBox,
    Xray,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Withheld {
    Version,
    Grammar,
    Oversize,
}
#[derive(Clone, Copy)]
enum Template {
    SingStarted(i32),
    SingMissingInterface,
    XrayStarted,
    XrayLocalDns,
}
/// Закрытые поля: никакой Debug/Serialize и строк из core.
pub(super) struct Detail {
    template: Template,
}
impl Detail {
    pub(super) fn body(&self) -> String {
        match self.template {
            Template::SingStarted(n) => {
                format!("sing-box started ({}.{:02}s)", n / 100, n % 100)
            }
            Template::SingMissingInterface => "missing default interface".into(),
            Template::XrayStarted => "Xray 26.10.31 started".into(),
            Template::XrayLocalDns => "DNS: created localhost client".into(),
        }
    }
    /// Предложение numeric codes, не изменение EventCode/disk.
    pub(super) fn projection(&self) -> (u8, Option<i32>) {
        (
            match self.template {
                Template::SingStarted(_) => 7,
                Template::SingMissingInterface => 8,
                Template::XrayStarted => 9,
                Template::XrayLocalDns => 10,
            },
            match self.template {
                Template::SingStarted(n) => Some(n),
                _ => None,
            },
        )
    }
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}
fn sing_prefix<'a>(s: &'a str, level: &str, colored: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(level).or_else(|| s.strip_prefix(colored))?;
    // Без timestamp файловый output не содержит elapsed; stderr сохраняет [ssss].
    if let Some(body) = rest.strip_prefix(' ') {
        return Some(body);
    }
    let (seconds, body) = rest.strip_prefix('[')?.split_once("] ")?;
    if !(4..=10).contains(&seconds.len()) || !digits(seconds) {
        return None;
    }
    Some(body)
}
fn xray_prefix(s: &str) -> Option<&str> {
    let date = s.as_bytes().get(..27)?;
    for (i, b) in date.iter().enumerate() {
        let expected = match i {
            4 | 7 => Some(b'/'),
            10 | 26 => Some(b' '),
            13 | 16 => Some(b':'),
            19 => Some(b'.'),
            _ => None,
        };
        if expected.map_or(!b.is_ascii_digit(), |e| *b != e) {
            return None;
        }
    }
    let year: u32 = s.get(..4)?.parse().ok()?;
    let month: usize = s.get(5..7)?.parse().ok()?;
    let day: u32 = s.get(8..10)?.parse().ok()?;
    let hour: u32 = s.get(11..13)?.parse().ok()?;
    let minute: u32 = s.get(14..16)?.parse().ok()?;
    let second: u32 = s.get(17..19)?.parse().ok()?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if year == 0
        || !(1..=12).contains(&month)
        || day == 0
        || day > days[month - 1]
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    s.get(27..)
}
fn elapsed(body: &str) -> Option<i32> {
    let s = body
        .strip_prefix("sing-box started (")?
        .strip_suffix("s)")?;
    let (whole, fraction) = s.split_once('.')?;
    // F.Seconds(time.Since(createdAt).Seconds()) в pinned sing: ровно две цифры.
    // Верхний предел 24 часа — intentional withholding, не принятие любого числа.
    if !digits(whole)
        || whole.len() > 5
        || (whole.len() > 1 && whole.starts_with('0'))
        || fraction.len() != 2
        || !digits(fraction)
    {
        return None;
    }
    let n = whole
        .parse::<i32>()
        .ok()?
        .checked_mul(100)?
        .checked_add(fraction.parse::<i32>().ok()?)?;
    (n <= 8_640_000).then_some(n)
}
pub(super) fn classify(core: Core, version: &str, line: &[u8]) -> Result<Detail, Withheld> {
    let known = match core {
        Core::SingBox => "1.14.4",
        Core::Xray => "26.10.31",
    };
    if version != known {
        return Err(Withheld::Version);
    }
    if line.len() > MAX_LINE {
        return Err(Withheld::Oversize);
    }
    let s = std::str::from_utf8(line).map_err(|_| Withheld::Grammar)?;
    let s = s.strip_suffix('\n').ok_or(Withheld::Grammar)?;
    let template = match core {
        Core::SingBox => {
            if let Some(body) = sing_prefix(s, "INFO", "\x1b[36mINFO\x1b[0m") {
                Template::SingStarted(elapsed(body).ok_or(Withheld::Grammar)?)
            } else if sing_prefix(s, "ERROR", "\x1b[31mERROR\x1b[0m")
                == Some("network: missing default interface")
            {
                Template::SingMissingInterface
            } else {
                return Err(Withheld::Grammar);
            }
        }
        Core::Xray => match xray_prefix(s) {
            Some("[Warning] core: Xray 26.10.31 started") => Template::XrayStarted,
            Some("[Info] app/dns: DNS: created localhost client") => Template::XrayLocalDns,
            _ => return Err(Withheld::Grammar),
        },
    };
    Ok(Detail { template })
}

#[derive(Default)]
pub(super) struct Counters {
    pub(super) withheld: u64,
    pub(super) oversize: u64,
    pub(super) dropped: u64,
}
/// Pure framing state; ни I/O, ни task/timer, ни встроенной очереди.
pub(super) struct Framer {
    bytes: [u8; MAX_LINE],
    len: usize,
    discarding: bool,
    stopped: bool,
    pub(super) counters: Counters,
}
impl Framer {
    pub(super) fn new() -> Self {
        Self {
            bytes: [0; MAX_LINE],
            len: 0,
            discarding: false,
            stopped: false,
            counters: Counters::default(),
        }
    }
    pub(super) fn stop(&mut self) {
        self.bytes.fill(0);
        self.len = 0;
        self.discarding = false;
        self.stopped = true;
    }
    /// Возвращает consumed; caller обязан yield перед продолжением. Offer — try-only.
    pub(super) fn feed(
        &mut self,
        core: Core,
        version: &str,
        input: &[u8],
        mut offer: impl FnMut(Detail) -> bool,
    ) -> usize {
        if self.stopped {
            return 0;
        }
        let mut lines = 0;
        let mut consumed = 0;
        for &b in input.iter().take(MAX_TURN) {
            if lines == 32 {
                break;
            }
            consumed += 1;
            if self.discarding {
                if b == b'\n' {
                    self.discarding = false;
                    lines += 1;
                }
                continue;
            }
            if self.len == MAX_LINE {
                self.bytes.fill(0);
                self.len = 0;
                self.counters.oversize = self.counters.oversize.saturating_add(1);
                self.discarding = b != b'\n';
                if b == b'\n' {
                    lines += 1;
                }
                continue;
            }
            self.bytes[self.len] = b;
            self.len += 1;
            if b == b'\n' {
                match classify(core, version, &self.bytes[..self.len]) {
                    Ok(detail) => {
                        if !offer(detail) {
                            self.counters.dropped = self.counters.dropped.saturating_add(1);
                        }
                    }
                    Err(_) => self.counters.withheld = self.counters.withheld.saturating_add(1),
                }
                self.bytes[..self.len].fill(0);
                self.len = 0;
                lines += 1;
            }
        }
        consumed
    }
}
impl Drop for Framer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SING: &[u8] = b"\x1b[36mINFO\x1b[0m[0000] sing-box started (0.00s)\n";
    const XRAY: &[u8] = b"2026/11/12 04:47:51.265312 [Warning] core: Xray 26.10.31 started\n";

    #[test]
    fn startup_exact_body() {
        let s = classify(Core::SingBox, "1.14.4", SING).expect("retained startup");
        assert_eq!(s.body(), "sing-box started (0.00s)");
        let x = classify(Core::Xray, "26.10.31", XRAY).expect("retained startup");
        assert_eq!(x.body(), "Xray 26.10.31 started");
    }

    #[test]
    fn retained_runtime_constants() {
        for (core, version, frame, body, code) in [
            (
                Core::SingBox,
                "1.14.4",
                b"\x1b[31mERROR\x1b[0m[0000] network: missing default interface\n".as_slice(),
                "missing default interface",
                8,
            ),
            (
                Core::Xray,
                "26.10.31",
                b"2026/11/12 01:12:49.716889 [Info] app/dns: DNS: created localhost client\n"
                    .as_slice(),
                "DNS: created localhost client",
                10,
            ),
        ] {
            let d = classify(core, version, frame).expect("retained frame");
            assert_eq!(d.body(), body);
            assert_eq!(d.projection(), (code, None));
        }
    }
    #[test]
    fn elapsed_roundtrip_and_closed_bounds() {
        for n in [0, 1, 9, 10, 99, 100, 101, 999, 10000, 8_640_000] {
            let body = format!("sing-box started ({}.{:02}s)", n / 100, n % 100);
            let frame = format!("INFO[0001] {body}\n");
            let d = classify(Core::SingBox, "1.14.4", frame.as_bytes()).expect("bounded time");
            assert_eq!(d.body(), body);
            assert_eq!(d.projection(), (7, Some(n)));
        }
        for value in [
            "-1.00",
            "01.00",
            "0.1",
            "0.000",
            "NaN",
            "inf",
            "86400.01",
            "999999999999.00",
            "CANARY",
        ] {
            let frame = format!("INFO[0000] sing-box started ({value}s)\n");
            assert!(classify(Core::SingBox, "1.14.4", frame.as_bytes()).is_err());
        }
    }
    #[test]
    fn no_secret_or_control_can_extend_a_template() {
        for (core, version, frame) in [
            (Core::SingBox, "1.14.4", SING),
            (Core::Xray, "26.10.31", XRAY),
        ] {
            assert!(matches!(
                classify(core, "unknown", frame),
                Err(Withheld::Version)
            ));
            for extra in [
                b"CANARY".as_slice(),
                b"<b>x</b>",
                b"\x1b",
                b"\0",
                b"\r",
                b"\t",
                b"\x7f",
                b"\xc2\x85",
                b"\xe2\x80\xae",
                b"\xff",
                b"\n",
            ] {
                for at in [0, frame.len() - 1, frame.len() / 2] {
                    let mut bad = frame.to_vec();
                    bad.splice(at..at, extra.iter().copied());
                    assert!(classify(core, version, &bad).is_err());
                }
            }
            assert!(classify(core, version, &frame[..frame.len() - 1]).is_err());
        }
        assert!(matches!(
            classify(Core::SingBox, "1.14.4", &[b'x'; MAX_LINE + 1]),
            Err(Withheld::Oversize)
        ));
    }
    #[test]
    fn timestamp_calendar_and_prefix_are_closed() {
        for (from, to) in [
            ("2026/11/12", "2026/02/29"),
            ("2026/11/12", "0000/11/12"),
            ("2026/11/12", "2026/13/12"),
            ("04:47:51", "24:47:51"),
            ("04:47:51", "04:60:51"),
            ("04:47:51", "04:47:60"),
            ("[Warning]", "[Info]"),
            ("26.10.31", "26.10.32"),
        ] {
            let s = std::str::from_utf8(XRAY)
                .expect("ASCII fixture")
                .replace(from, to);
            assert!(classify(Core::Xray, "26.10.31", s.as_bytes()).is_err());
        }
        let leap = std::str::from_utf8(XRAY)
            .expect("ASCII fixture")
            .replace("2026/11/12", "2024/02/29");
        assert!(classify(Core::Xray, "26.10.31", leap.as_bytes()).is_ok());
    }
    #[test]
    fn fragments_oversize_resync_loss_and_stop() {
        let mut f = Framer::new();
        let mut bodies = Vec::new();
        for b in SING {
            assert_eq!(
                f.feed(Core::SingBox, "1.14.4", &[*b], |d| {
                    bodies.push(d.body());
                    true
                }),
                1
            );
        }
        assert_eq!(bodies, ["sing-box started (0.00s)"]);
        let mut input = vec![b'x'; MAX_LINE + 1];
        input.extend_from_slice(b"\n");
        input.extend_from_slice(SING);
        f.feed(Core::SingBox, "1.14.4", &input, |_| false);
        assert_eq!(f.counters.oversize, 1);
        assert_eq!(f.counters.dropped, 1);
        f.feed(Core::SingBox, "unknown", SING, |_| {
            panic!("version withheld")
        });
        assert_eq!(f.counters.withheld, 1);
        f.feed(Core::SingBox, "1.14.4", b"CANARY", |_| panic!("partial"));
        f.stop();
        assert_eq!(f.len, 0);
        assert!(f.bytes.iter().all(|b| *b == 0));
        assert_eq!(
            f.feed(Core::SingBox, "1.14.4", SING, |_| panic!("stopped")),
            0
        );
    }
    #[test]
    fn turn_and_counter_bounds() {
        let mut f = Framer::new();
        let input = SING.repeat(33);
        let mut offers = 0;
        let n = f.feed(Core::SingBox, "1.14.4", &input, |_| {
            offers += 1;
            false
        });
        assert_eq!(n, SING.len() * 32);
        assert_eq!(offers, 32);
        assert_eq!(f.counters.dropped, 32);
        assert_eq!(
            f.feed(Core::SingBox, "1.14.4", &input[n..], |_| true),
            SING.len()
        );
        f.counters.dropped = u64::MAX;
        f.feed(Core::SingBox, "1.14.4", SING, |_| false);
        assert_eq!(f.counters.dropped, u64::MAX);
        assert_eq!(
            f.feed(Core::SingBox, "1.14.4", &[b'x'; MAX_TURN + 1], |_| false),
            MAX_TURN
        );
    }

    #[test]
    fn oversize_discards_through_lf_and_counters_saturate() {
        let mut f = Framer::new();
        f.counters.oversize = u64::MAX;
        f.feed(Core::SingBox, "1.14.4", &[b'x'; MAX_LINE + 1], |_| {
            panic!("oversize")
        });
        assert!(f.discarding);
        assert!(f.bytes.iter().all(|b| *b == 0));
        let mut offers = 0;
        f.feed(Core::SingBox, "1.14.4", SING, |_| {
            offers += 1;
            true
        });
        assert_eq!(offers, 0); // SING — хвост oversized frame до LF, не новое событие.
        f.feed(Core::SingBox, "1.14.4", SING, |_| {
            offers += 1;
            true
        });
        assert_eq!(offers, 1);
        assert_eq!(f.counters.oversize, u64::MAX);
        f.counters.withheld = u64::MAX;
        f.feed(Core::SingBox, "1.14.4", b"CANARY\n", |_| panic!("unknown"));
        assert_eq!(f.counters.withheld, u64::MAX);
        assert!(f.bytes.iter().all(|b| *b == 0));
    }

    #[test]
    fn file_output_info_without_timestamp() {
        for prefix in ["INFO ", "\x1b[36mINFO\x1b[0m "] {
            let frame = format!("{prefix}sing-box started (0.01s)\n");
            let d =
                classify(Core::SingBox, "1.14.4", frame.as_bytes()).expect("file output prefix");
            assert_eq!(d.body(), "sing-box started (0.01s)");
        }
    }
}

/// Повторная проверка числовой проекции перед UI/export/recovery.
pub(super) fn project(source: EventSource, code: EventCode, value: Option<i32>) -> Option<Detail> {
    if source != EventSource::Engine {
        return None;
    }
    let template = match (code, value) {
        (EventCode::NativeSingStarted, Some(n @ 0..=8_640_000)) => Template::SingStarted(n),
        (EventCode::NativeSingMissingInterface, None) => Template::SingMissingInterface,
        (EventCode::NativeXrayStarted, None) => Template::XrayStarted,
        (EventCode::NativeXrayLocalDns, None) => Template::XrayLocalDns,
        _ => return None,
    };
    Some(Detail { template })
}
/// Этот объект создаётся владельцем child; поля scope не принимаются из stdout.
pub(crate) struct Input {
    framer: Framer,
    log: EventLog,
    core: Core,
    version: &'static str,
}
impl Input {
    pub(crate) fn new(engine: yonik_engine::Engine, log: EventLog) -> Self {
        Self {
            framer: Framer::new(),
            core: match engine {
                yonik_engine::Engine::SingBox => Core::SingBox,
                yonik_engine::Engine::Xray => Core::Xray,
            },
            version: engine.version(),
            log,
        }
    }
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> usize {
        let before = (self.framer.counters.withheld, self.framer.counters.oversize);
        let log = &self.log;
        let n = self.framer.feed(self.core, self.version, bytes, |detail| {
            let (code, value) = detail.projection();
            let code = match code {
                7 => EventCode::NativeSingStarted,
                8 => EventCode::NativeSingMissingInterface,
                9 => EventCode::NativeXrayStarted,
                10 => EventCode::NativeXrayLocalDns,
                _ => return false,
            };
            log.try_record(EventSource::Engine, code, value)
        });
        log.reject_count(
            self.framer
                .counters
                .withheld
                .saturating_sub(before.0)
                .saturating_add(self.framer.counters.oversize.saturating_sub(before.1)),
        );
        n
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        if self.framer.len != 0 {
            self.log.reject();
        }
    }
}

#[cfg(test)]
mod producer_tests {
    use super::*;
    use crate::event_log::{EventContext, LogFilter, MAX_PAGE};

    #[test]
    fn immutable_scope_closed_projection_busy_and_canaries() {
        let root = EventLog::new();
        let old = root.with_context(EventContext {
            session_id: 1,
            generation: 7,
        });
        let mut input = Input::new(yonik_engine::Engine::SingBox, old.clone());
        let _new = old.with_generation(8);
        input.feed(b"INFO[0000] sing-box started (1.23s)\n");
        input.feed(b"CANARY raw config/path/URL\x1b\0\r\n");
        let page = root.read_scoped(0, LogFilter::default(), MAX_PAGE).unwrap();
        assert_eq!(
            page.contexts,
            [Some(EventContext {
                session_id: 1,
                generation: 7
            })]
        );
        assert_eq!(
            page.page.events[0].technical_details().as_deref(),
            Some("sing-box started (1.23s)")
        );
        assert!(!page.export().contains("CANARY"));
        assert!(!page.export().contains('\x1b'));
        assert_eq!(page.page.rejected, 1);
        let lock = root.0.ring.lock().unwrap();
        input.feed(b"INFO[0000] sing-box started (1.23s)\n");
        drop(lock);
        assert_eq!(
            root.read(0, LogFilter::default(), MAX_PAGE)
                .unwrap()
                .rejected,
            2
        );
        assert!(!root.try_record(EventSource::Engine, EventCode::NativeSingStarted, Some(-1)));
        assert!(!old.try_record(EventSource::App, EventCode::NativeXrayStarted, None));
        assert!(!old.try_record(EventSource::Engine, EventCode::NativeXrayStarted, Some(1)));
        input.feed(b"partial CANARY");
        drop(input);
        assert_eq!(
            root.read(0, LogFilter::default(), MAX_PAGE)
                .unwrap()
                .rejected,
            6
        );
    }
}
