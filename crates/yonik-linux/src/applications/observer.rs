//! События только test-support; пустой контекст без feature не хранит состояние.
#[allow(dead_code)]
// Status зарезервирован закрытым IO-классификатором; сейчас не читается.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadClass {
    Dir,
    Stat,
    Exe,
    Desktop,
    Header,
    Status,
}
#[cfg(feature = "test-support")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Admitted { token: u64, processes: bool },
    Read { token: u64, class: ReadClass },
    Released { token: u64 },
}
#[cfg(feature = "test-support")]
#[derive(Clone)]
pub struct Observer(std::sync::Arc<ObserverInner>);
#[cfg(feature = "test-support")]
struct ObserverInner {
    next: std::sync::atomic::AtomicU64,
    callback: Box<dyn Fn(Event) + Send + Sync>,
}
#[cfg(feature = "test-support")]
impl Observer {
    pub fn new(callback: impl Fn(Event) + Send + Sync + 'static) -> Self {
        Self(std::sync::Arc::new(ObserverInner {
            next: std::sync::atomic::AtomicU64::new(1),
            callback: Box::new(callback),
        }))
    }
}
#[cfg(feature = "test-support")]
struct Session {
    observer: Observer,
    token: u64,
}
#[cfg(feature = "test-support")]
impl Drop for Session {
    fn drop(&mut self) {
        (self.observer.0.callback)(Event::Released { token: self.token });
    }
}
#[derive(Default)]
pub(super) struct Io {
    #[cfg(feature = "test-support")]
    session: Option<Session>,
}
impl Io {
    #[cfg(feature = "test-support")]
    pub(super) fn new(observer: Option<&Observer>, processes: bool) -> Self {
        let session = observer.map(|observer| {
            let token = observer
                .0
                .next
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // Guard существует до callback, включая раскрутку стека при panic.
            let session = Session {
                observer: observer.clone(),
                token,
            };
            (observer.0.callback)(Event::Admitted { token, processes });
            session
        });
        Self { session }
    }
    pub(super) fn read(&self, class: ReadClass) {
        #[cfg(feature = "test-support")]
        if let Some(session) = &self.session {
            (session.observer.0.callback)(Event::Read {
                token: session.token,
                class,
            });
        }
        #[cfg(not(feature = "test-support"))]
        let _ = class;
    }
}
