use std::{
    backtrace::Backtrace,
    cell::{Cell, RefCell},
    error::Error,
    fmt,
    future::Future,
    io::{self, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{
        Arc, Mutex, MutexGuard, Once,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll, Waker},
    thread,
    time::Duration,
};

use iocraft::prelude::*;

#[cfg(test)]
use std::sync::atomic::AtomicUsize;

use crate::{NO_STYLE, YELLOW, is_terminal_available, runtime::block_on};

const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const FRAME_INTERVAL: Duration = Duration::from_millis(80);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_WAITER: AtomicU64 = AtomicU64::new(1);
static INSTALL_PANIC_HOOK: Once = Once::new();
#[cfg(test)]
static ACTIVE_TIMER_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Shared cancellation context passed unchanged to a spinner action.
///
/// Cancellation is cooperative for the action. Interactive presentation stops
/// as soon as cancellation wins, while an action that ignores this token keeps
/// running on its worker thread like the oracle's goroutine.
#[derive(Clone, Default)]
pub struct CancellationToken {
    signal: Arc<Signal>,
}

impl CancellationToken {
    /// Creates an uncancelled context.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels this context. Repeated calls preserve the first cancellation.
    pub fn cancel(&self) {
        self.signal.fire();
    }

    /// Reports whether this context has been cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.signal.sequence() != 0
    }

    #[cfg(test)]
    fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.signal, &other.signal)
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("is_cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Default)]
struct Interruption {
    signal: Arc<Signal>,
}

impl Interruption {
    fn interrupt(&self) {
        self.signal.fire();
    }

    #[cfg(test)]
    fn is_interrupted(&self) -> bool {
        self.signal.sequence() != 0
    }
}

/// Future returned by [`run_spinner`].
#[must_use = "futures do nothing unless polled or awaited"]
pub struct Spinner<T, E> {
    title: String,
    cancellation: CancellationToken,
    interruption: Interruption,
    dropped: Arc<Signal>,
    action: Option<Action<T, E>>,
    mode: Mode,
    output: Option<Box<dyn Write + Send>>,
    state: SpinnerState<T, E>,
}

/// Error returned by an interactive spinner.
#[derive(Debug)]
pub enum SpinnerError<E> {
    /// The caller-provided action failed.
    Action(E),
    /// The caller's cancellation context won the race.
    Cancelled,
    /// Exact Control+C interrupted presentation.
    Interrupted,
    /// A worker or timer could not be started.
    Io(io::Error),
    /// The interactive action panicked after its diagnostic was emitted.
    ActionPanicked,
    /// The spinner's internal driver panicked.
    DriverPanicked,
}

impl<E: fmt::Display> fmt::Display for SpinnerError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Action(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("spinner cancelled"),
            Self::Interrupted => formatter.write_str("spinner interrupted"),
            Self::Io(error) => error.fmt(formatter),
            Self::ActionPanicked => formatter.write_str("spinner action panicked"),
            Self::DriverPanicked => formatter.write_str("spinner driver panicked"),
        }
    }
}

impl<E> Error for SpinnerError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Action(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Cancelled | Self::Interrupted | Self::ActionPanicked | Self::DriverPanicked => {
                None
            }
        }
    }
}

/// Runs an action with a terminal spinner and an externally cancellable context.
///
/// Interactive actions run on a named worker thread after terminal setup. In
/// non-terminal mode, the title write is best-effort, then the action runs
/// synchronously in the polling caller's context and only its result is
/// returned.
pub fn run_spinner<F, T, E>(
    cancellation: CancellationToken,
    title: impl Into<String>,
    action: F,
) -> Spinner<T, E>
where
    F: FnOnce(CancellationToken) -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
{
    new_spinner(
        cancellation,
        title.into(),
        action,
        Mode::detected(),
        io::stderr(),
    )
}

impl<T, E> Future for Spinner<T, E>
where
    T: Send + 'static,
    E: Send + 'static,
{
    type Output = Result<T, SpinnerError<E>>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            match &mut this.state {
                SpinnerState::New => {
                    if this.mode.is_interactive() && this.cancellation.is_cancelled() {
                        this.state = SpinnerState::Done;
                        return Poll::Ready(Err(SpinnerError::Cancelled));
                    }

                    if !this.mode.is_interactive() {
                        if let Some(output) = this.output.as_mut() {
                            let _ = writeln!(output, "{}", this.title);
                        }
                        let action = this.action.take().expect("spinner action starts once");
                        this.state = SpinnerState::Done;
                        return Poll::Ready(
                            action(this.cancellation.clone()).map_err(SpinnerError::Action),
                        );
                    }

                    install_spinner_panic_hook();
                    let action_state = Arc::new(ActionState::default());
                    let action = this.action.take().expect("spinner action starts once");
                    let action_context = this.cancellation.clone();
                    let worker_cancellation = action_context.signal.clone();
                    let worker_state = action_state.clone();
                    let start_action = Arc::new(Signal::default());
                    let worker_start = start_action.clone();
                    let worker_dropped = this.dropped.clone();
                    if let Err(error) = thread::Builder::new()
                        .name("ployz-spinner-action".to_owned())
                        .spawn(move || {
                            block_on(WaitForAny::new([
                                worker_start.clone(),
                                worker_cancellation.clone(),
                                worker_dropped.clone(),
                            ]));
                            if !action_is_admitted(
                                &worker_start,
                                &worker_cancellation,
                                &worker_dropped,
                            ) {
                                return;
                            }
                            let completion =
                                run_interactive_action(action, action_context, io::stderr());
                            *locked(&worker_state.result) = Some(completion);
                            worker_state.signal.fire();
                        })
                    {
                        this.state = SpinnerState::Done;
                        return Poll::Ready(Err(SpinnerError::Io(error)));
                    }

                    let completion = Arc::new(Completion::default());
                    let setup_failure = Arc::new(Signal::default());
                    let driver = Driver {
                        title: this.title.clone(),
                        cancellation: this.cancellation.clone(),
                        interruption: this.interruption.clone(),
                        dropped: this.dropped.clone(),
                        action: action_state,
                        start_action,
                        setup_failure,
                        completion: completion.clone(),
                        mode: this.mode,
                    };
                    let panic_completion = completion.clone();
                    let panic_dropped = this.dropped.clone();
                    match thread::Builder::new()
                        .name("ployz-spinner-driver".to_owned())
                        .spawn(move || {
                            if catch_unwind(AssertUnwindSafe(|| driver.run())).is_err() {
                                panic_dropped.fire();
                                panic_completion.complete(Err(SpinnerError::DriverPanicked));
                            }
                        }) {
                        Ok(_) => this.state = SpinnerState::Interactive(completion),
                        Err(error) => {
                            this.dropped.fire();
                            this.state = SpinnerState::Done;
                            return Poll::Ready(Err(SpinnerError::Io(error)));
                        }
                    }
                }
                SpinnerState::Interactive(completion) => {
                    if let Poll::Ready(output) = completion.poll(context) {
                        this.state = SpinnerState::Done;
                        return Poll::Ready(output);
                    }
                    return Poll::Pending;
                }
                SpinnerState::Done => panic!("polled Spinner after completion"),
            }
        }
    }
}

impl<T, E> Drop for Spinner<T, E> {
    fn drop(&mut self) {
        if matches!(self.state, SpinnerState::Interactive(_)) {
            self.dropped.fire();
        }
    }
}

type Action<T, E> = Box<dyn FnOnce(CancellationToken) -> Result<T, E> + Send + 'static>;

enum SpinnerState<T, E> {
    New,
    Interactive(Arc<Completion<T, E>>),
    Done,
}

#[derive(Clone, Copy)]
enum Mode {
    Plain,
    Terminal,
    #[cfg(test)]
    SimulatedTerminal,
    #[cfg(test)]
    TimerFailure,
    #[cfg(test)]
    DriverPanic,
    #[cfg(test)]
    RenderFailure,
    #[cfg(test)]
    CancellationBeforeAdmission,
    #[cfg(test)]
    TerminalSetupFailure,
}

impl Mode {
    fn detected() -> Self {
        if is_terminal_available() {
            Self::Terminal
        } else {
            Self::Plain
        }
    }

    const fn is_interactive(self) -> bool {
        !matches!(self, Self::Plain)
    }
}

fn new_spinner<F, W, T, E>(
    cancellation: CancellationToken,
    title: String,
    action: F,
    mode: Mode,
    output: W,
) -> Spinner<T, E>
where
    F: FnOnce(CancellationToken) -> Result<T, E> + Send + 'static,
    W: Write + Send + 'static,
{
    Spinner {
        title,
        cancellation,
        interruption: Interruption::default(),
        dropped: Arc::new(Signal::default()),
        action: Some(Box::new(action)),
        mode,
        output: Some(Box::new(output)),
        state: SpinnerState::New,
    }
}

struct Driver<T, E> {
    title: String,
    cancellation: CancellationToken,
    interruption: Interruption,
    dropped: Arc<Signal>,
    action: Arc<ActionState<T, E>>,
    start_action: Arc<Signal>,
    setup_failure: Arc<Signal>,
    completion: Arc<Completion<T, E>>,
    mode: Mode,
}

impl<T, E> Driver<T, E>
where
    T: Send + 'static,
    E: Send + 'static,
{
    fn run(self) {
        #[cfg(test)]
        let track_timer = matches!(self.mode, Mode::DriverPanic);
        #[cfg(not(test))]
        let track_timer = false;
        #[cfg(test)]
        let clock = if matches!(self.mode, Mode::TimerFailure) {
            Err(io::Error::other("injected timer startup failure"))
        } else {
            FrameClock::start(track_timer)
        };
        #[cfg(not(test))]
        let clock = FrameClock::start(track_timer);

        let clock = match clock {
            Ok(clock) => clock,
            Err(error) => {
                self.dropped.fire();
                self.completion.complete(Err(SpinnerError::Io(error)));
                return;
            }
        };
        let _clock_shutdown = ClockShutdown(clock.clone());

        #[cfg(test)]
        if matches!(self.mode, Mode::DriverPanic) {
            panic!("injected spinner driver panic after timer startup");
        }

        let render_result = match self.mode {
            Mode::Plain => unreachable!("plain spinners do not use the terminal driver"),
            Mode::Terminal => {
                let mut view = element!(SpinnerView(
                    title: self.title.clone(),
                    action: self.action.signal.clone(),
                    start_action: self.start_action.clone(),
                    setup_failure: self.setup_failure.clone(),
                    force_setup_failure: false,
                    cancellation: self.cancellation.signal.clone(),
                    interruption: self.interruption.clone(),
                    dropped: self.dropped.clone(),
                    clock: clock.clone(),
                ));
                block_on(view.render_loop().output(Output::Stderr).ignore_ctrl_c())
            }
            #[cfg(test)]
            Mode::SimulatedTerminal => {
                self.start_action.fire();
                block_on(self.wait_for_authoritative_outcome());
                Ok(())
            }
            #[cfg(test)]
            Mode::TimerFailure | Mode::DriverPanic => unreachable!(),
            #[cfg(test)]
            Mode::RenderFailure => {
                // This mode models a renderer that initialized successfully,
                // admitted the action, and then lost its output stream.
                self.start_action.fire();
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "injected render failure",
                ))
            }
            #[cfg(test)]
            Mode::CancellationBeforeAdmission => {
                self.cancellation.cancel();
                self.start_action.fire();
                block_on(self.wait_for_authoritative_outcome());
                Ok(())
            }
            #[cfg(test)]
            Mode::TerminalSetupFailure => {
                let mut view = element!(SpinnerView(
                    title: self.title.clone(),
                    action: self.action.signal.clone(),
                    start_action: self.start_action.clone(),
                    setup_failure: self.setup_failure.clone(),
                    force_setup_failure: true,
                    cancellation: self.cancellation.signal.clone(),
                    interruption: self.interruption.clone(),
                    dropped: self.dropped.clone(),
                    clock: clock.clone(),
                ));
                block_on(
                    view.render_loop()
                        .output(Output::Stderr)
                        .stderr(io::sink())
                        .ignore_ctrl_c(),
                )
            }
        };
        if let Err(error) = render_result.as_ref()
            && self.start_action.sequence() == 0
        {
            self.dropped.fire();
            self.completion
                .complete(Err(SpinnerError::Io(io::Error::new(
                    error.kind(),
                    error.to_string(),
                ))));
            return;
        }
        if render_result.is_err()
            && first_signal(
                &self.action.signal,
                &self.cancellation.signal,
                &self.interruption.signal,
                &self.setup_failure,
                &self.dropped,
            )
            .is_none()
        {
            #[cfg(test)]
            let fallback_result = if matches!(self.mode, Mode::RenderFailure) {
                Err(io::Error::other("injected fallback failure"))
            } else {
                self.run_silent_fallback()
            };
            #[cfg(not(test))]
            let fallback_result = self.run_silent_fallback();

            if let Err(error) = fallback_result
                && first_signal(
                    &self.action.signal,
                    &self.cancellation.signal,
                    &self.interruption.signal,
                    &self.setup_failure,
                    &self.dropped,
                )
                .is_none()
            {
                if self.start_action.sequence() == 0 {
                    self.dropped.fire();
                    self.completion.complete(Err(SpinnerError::Io(error)));
                    return;
                }
                block_on(self.wait_for_authoritative_outcome());
            }
        }

        let winner = first_signal(
            &self.action.signal,
            &self.cancellation.signal,
            &self.interruption.signal,
            &self.setup_failure,
            &self.dropped,
        );
        if winner == Some(Winner::Dropped) {
            return;
        }

        let output = match winner {
            Some(Winner::Action) => match locked(&self.action.result).take() {
                Some(ActionCompletion::Output(output)) => output.map_err(SpinnerError::Action),
                Some(ActionCompletion::Panicked) => Err(SpinnerError::ActionPanicked),
                None => Err(SpinnerError::DriverPanicked),
            },
            Some(Winner::Cancelled) => Err(SpinnerError::Cancelled),
            Some(Winner::Interrupted) => Err(SpinnerError::Interrupted),
            Some(Winner::SetupFailed) => {
                self.dropped.fire();
                Err(SpinnerError::Io(io::Error::other(
                    "terminal input initialization failed",
                )))
            }
            Some(Winner::Dropped) => unreachable!(),
            None => Err(SpinnerError::DriverPanicked),
        };
        self.completion.complete(output);
    }

    fn wait_for_authoritative_outcome(&self) -> WaitForAny<5> {
        WaitForAny::new([
            self.action.signal.clone(),
            self.cancellation.signal.clone(),
            self.interruption.signal.clone(),
            self.setup_failure.clone(),
            self.dropped.clone(),
        ])
    }

    fn run_silent_fallback(&self) -> io::Result<()> {
        #[cfg(test)]
        let force_setup_failure = matches!(self.mode, Mode::TerminalSetupFailure);
        #[cfg(not(test))]
        let force_setup_failure = false;
        let mut view = element!(SpinnerWaitView(
            action: self.action.signal.clone(),
            start_action: self.start_action.clone(),
            setup_failure: self.setup_failure.clone(),
            force_setup_failure,
            cancellation: self.cancellation.signal.clone(),
            interruption: self.interruption.clone(),
            dropped: self.dropped.clone(),
        ));
        block_on(
            view.render_loop()
                .output(Output::Stderr)
                .stderr(io::sink())
                .ignore_ctrl_c(),
        )
    }
}

struct ActionState<T, E> {
    signal: Arc<Signal>,
    result: Mutex<Option<ActionCompletion<T, E>>>,
}

impl<T, E> Default for ActionState<T, E> {
    fn default() -> Self {
        Self {
            signal: Arc::new(Signal::default()),
            result: Mutex::new(None),
        }
    }
}

enum ActionCompletion<T, E> {
    Output(Result<T, E>),
    Panicked,
}

struct Completion<T, E> {
    output: Mutex<Option<Result<T, SpinnerError<E>>>>,
    waker: Mutex<Option<Waker>>,
}

impl<T, E> Default for Completion<T, E> {
    fn default() -> Self {
        Self {
            output: Mutex::new(None),
            waker: Mutex::new(None),
        }
    }
}

impl<T, E> Completion<T, E> {
    fn complete(&self, output: Result<T, SpinnerError<E>>) {
        let mut stored = locked(&self.output);
        if stored.is_some() {
            return;
        }
        *stored = Some(output);
        drop(stored);
        if let Some(waker) = locked(&self.waker).take() {
            waker.wake();
        }
    }

    fn poll(&self, context: &mut Context<'_>) -> Poll<Result<T, SpinnerError<E>>> {
        if let Some(output) = locked(&self.output).take() {
            return Poll::Ready(output);
        }
        *locked(&self.waker) = Some(context.waker().clone());
        locked(&self.output)
            .take()
            .map_or(Poll::Pending, Poll::Ready)
    }
}

#[derive(Default)]
struct Signal {
    sequence: AtomicU64,
    wakers: Mutex<Vec<(u64, Waker)>>,
}

impl Signal {
    fn fire(&self) {
        let sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        if self
            .sequence
            .compare_exchange(0, sequence, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            for (_, waker) in locked(&self.wakers).drain(..) {
                waker.wake();
            }
        }
    }

    fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Acquire)
    }

    fn register(&self, waiter: u64, waker: &Waker) {
        if self.sequence() != 0 {
            waker.wake_by_ref();
            return;
        }
        let mut wakers = locked(&self.wakers);
        if self.sequence() != 0 {
            drop(wakers);
            waker.wake_by_ref();
        } else if let Some((_, registered)) = wakers.iter_mut().find(|(id, _)| *id == waiter) {
            registered.clone_from(waker);
        } else {
            wakers.push((waiter, waker.clone()));
        }
    }

    fn unregister(&self, waiter: u64) {
        locked(&self.wakers).retain(|(id, _)| *id != waiter);
    }

    #[cfg(test)]
    fn waiter_count(&self) -> usize {
        locked(&self.wakers).len()
    }
}

struct WaitForAny<const N: usize> {
    signals: [Arc<Signal>; N],
    waiter: u64,
}

impl<const N: usize> WaitForAny<N> {
    fn new(signals: [Arc<Signal>; N]) -> Self {
        Self {
            signals,
            waiter: NEXT_WAITER.fetch_add(1, Ordering::Relaxed),
        }
    }

    fn unregister(&self) {
        for signal in &self.signals {
            signal.unregister(self.waiter);
        }
    }
}

impl<const N: usize> Drop for WaitForAny<N> {
    fn drop(&mut self) {
        self.unregister();
    }
}

impl<const N: usize> Future for WaitForAny<N> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        if self.signals.iter().any(|signal| signal.sequence() != 0) {
            self.unregister();
            return Poll::Ready(());
        }
        for signal in &self.signals {
            signal.register(self.waiter, context.waker());
        }
        if self.signals.iter().any(|signal| signal.sequence() != 0) {
            self.unregister();
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Winner {
    Action,
    Cancelled,
    Interrupted,
    SetupFailed,
    Dropped,
}

fn first_signal(
    action: &Signal,
    cancellation: &Signal,
    interruption: &Signal,
    setup_failure: &Signal,
    dropped: &Signal,
) -> Option<Winner> {
    [
        (action.sequence(), Winner::Action),
        (cancellation.sequence(), Winner::Cancelled),
        (interruption.sequence(), Winner::Interrupted),
        (setup_failure.sequence(), Winner::SetupFailed),
        (dropped.sequence(), Winner::Dropped),
    ]
    .into_iter()
    .filter(|(sequence, _)| *sequence != 0)
    .min_by_key(|(sequence, _)| *sequence)
    .map(|(_, winner)| winner)
}

fn action_is_admitted(start: &Signal, cancellation: &Signal, dropped: &Signal) -> bool {
    let start_sequence = start.sequence();
    start_sequence != 0
        && [cancellation.sequence(), dropped.sequence()]
            .into_iter()
            .all(|sequence| sequence == 0 || start_sequence < sequence)
}

#[derive(Default)]
struct FrameClock {
    generation: AtomicU64,
    stopped: AtomicBool,
    wakers: Mutex<Vec<Waker>>,
}

struct ClockShutdown(Arc<FrameClock>);

impl Drop for ClockShutdown {
    fn drop(&mut self) {
        self.0.stop();
    }
}

impl FrameClock {
    fn start(track_for_test: bool) -> io::Result<Arc<Self>> {
        let clock = Arc::new(Self {
            generation: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            wakers: Mutex::new(Vec::new()),
        });
        let timer_clock = clock.clone();
        thread::Builder::new()
            .name("ployz-spinner-timer".to_owned())
            .spawn(move || {
                #[cfg(test)]
                if track_for_test {
                    ACTIVE_TIMER_THREADS.fetch_add(1, Ordering::SeqCst);
                }
                #[cfg(not(test))]
                let _ = track_for_test;
                struct TimerThreadFinished {
                    #[cfg(test)]
                    tracked: bool,
                }
                impl Drop for TimerThreadFinished {
                    fn drop(&mut self) {
                        #[cfg(test)]
                        if self.tracked {
                            ACTIVE_TIMER_THREADS.fetch_sub(1, Ordering::SeqCst);
                        }
                    }
                }
                let _finished = TimerThreadFinished {
                    #[cfg(test)]
                    tracked: track_for_test,
                };
                while !timer_clock.stopped.load(Ordering::Acquire) {
                    thread::sleep(FRAME_INTERVAL);
                    if timer_clock.stopped.load(Ordering::Acquire) {
                        break;
                    }
                    timer_clock.generation.fetch_add(1, Ordering::AcqRel);
                    for waker in locked(&timer_clock.wakers).drain(..) {
                        waker.wake();
                    }
                }
            })?;
        Ok(clock)
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    fn next_tick(self: &Arc<Self>, generation: u64) -> NextTick {
        NextTick {
            clock: self.clone(),
            generation,
        }
    }
}

struct NextTick {
    clock: Arc<FrameClock>,
    generation: u64,
}

impl Future for NextTick {
    type Output = u64;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<u64> {
        let generation = self.clock.generation.load(Ordering::Acquire);
        if generation != self.generation {
            return Poll::Ready(generation);
        }
        let mut wakers = locked(&self.clock.wakers);
        let generation = self.clock.generation.load(Ordering::Acquire);
        if generation != self.generation {
            Poll::Ready(generation)
        } else {
            if !wakers.iter().any(|waker| waker.will_wake(context.waker())) {
                wakers.push(context.waker().clone());
            }
            Poll::Pending
        }
    }
}

struct TerminalReadyHook {
    ready: Arc<AtomicBool>,
    force_setup_failure: bool,
}

impl Hook for TerminalReadyHook {
    fn post_component_update(&mut self, updater: &mut ComponentUpdater) {
        self.ready.store(
            !self.force_setup_failure && updater.is_terminal_raw_mode_enabled(),
            Ordering::Release,
        );
    }
}

#[derive(Default, Props)]
struct SpinnerViewProps {
    title: String,
    action: Arc<Signal>,
    start_action: Arc<Signal>,
    setup_failure: Arc<Signal>,
    force_setup_failure: bool,
    cancellation: Arc<Signal>,
    interruption: Interruption,
    dropped: Arc<Signal>,
    clock: Arc<FrameClock>,
}

#[component]
fn SpinnerView(mut hooks: Hooks, props: &SpinnerViewProps) -> impl Into<AnyElement<'static>> {
    let mut system = hooks.use_context_mut::<SystemContext>();
    let mut frame = hooks.use_state(|| 0usize);
    let mut finished = hooks.use_state(|| false);

    hooks.use_future({
        let signals = [
            props.action.clone(),
            props.cancellation.clone(),
            props.interruption.signal.clone(),
            props.setup_failure.clone(),
            props.dropped.clone(),
        ];
        async move {
            WaitForAny::new(signals).await;
            finished.set(true);
        }
    });
    hooks.use_future({
        let clock = props.clock.clone();
        async move {
            let mut generation = clock.generation.load(Ordering::Acquire);
            loop {
                generation = clock.next_tick(generation).await;
                frame.set((frame.get() + 1) % FRAMES.len());
            }
        }
    });

    let interruption = props.interruption.clone();
    hooks.use_terminal_events(move |event| {
        if is_interrupt_event(&event) {
            interruption.interrupt();
            finished.set(true);
        }
    });
    let terminal_ready = hooks
        .use_hook(|| TerminalReadyHook {
            ready: Arc::new(AtomicBool::new(false)),
            force_setup_failure: props.force_setup_failure,
        })
        .ready
        .clone();
    hooks.use_future({
        let start_action = props.start_action.clone();
        let setup_failure = props.setup_failure.clone();
        let cancellation = props.cancellation.clone();
        let dropped = props.dropped.clone();
        async move {
            if start_action.sequence() != 0 {
                return;
            }
            if terminal_ready.load(Ordering::Acquire) {
                if cancellation.sequence() == 0 && dropped.sequence() == 0 {
                    start_action.fire();
                }
            } else {
                setup_failure.fire();
            }
        }
    });

    if finished.get() {
        system.exit();
    }

    element! {
        MixedText(contents: vec![
            YELLOW.content(FRAMES[frame.get()]),
            NO_STYLE.content(format!(" {}", props.title)),
        ])
    }
}

#[derive(Default, Props)]
struct SpinnerWaitViewProps {
    action: Arc<Signal>,
    start_action: Arc<Signal>,
    setup_failure: Arc<Signal>,
    force_setup_failure: bool,
    cancellation: Arc<Signal>,
    interruption: Interruption,
    dropped: Arc<Signal>,
}

#[component]
fn SpinnerWaitView(
    mut hooks: Hooks,
    props: &SpinnerWaitViewProps,
) -> impl Into<AnyElement<'static>> {
    let mut system = hooks.use_context_mut::<SystemContext>();
    let mut finished = hooks.use_state(|| false);
    hooks.use_future({
        let signals = [
            props.action.clone(),
            props.cancellation.clone(),
            props.interruption.signal.clone(),
            props.setup_failure.clone(),
            props.dropped.clone(),
        ];
        async move {
            WaitForAny::new(signals).await;
            finished.set(true);
        }
    });

    let interruption = props.interruption.clone();
    hooks.use_terminal_events(move |event| {
        if is_interrupt_event(&event) {
            interruption.interrupt();
            finished.set(true);
        }
    });
    let terminal_ready = hooks
        .use_hook(|| TerminalReadyHook {
            ready: Arc::new(AtomicBool::new(false)),
            force_setup_failure: props.force_setup_failure,
        })
        .ready
        .clone();
    hooks.use_future({
        let start_action = props.start_action.clone();
        let setup_failure = props.setup_failure.clone();
        let cancellation = props.cancellation.clone();
        let dropped = props.dropped.clone();
        async move {
            if start_action.sequence() != 0 {
                return;
            }
            if terminal_ready.load(Ordering::Acquire) {
                if cancellation.sequence() == 0 && dropped.sequence() == 0 {
                    start_action.fire();
                }
            } else {
                setup_failure.fire();
            }
        }
    });
    if finished.get() {
        system.exit();
    }
    element!(View)
}

fn is_interrupt_event(event: &TerminalEvent) -> bool {
    matches!(
        event,
        TerminalEvent::Key(KeyEvent {
            code: KeyCode::Char('c' | 'C'),
            modifiers,
            ..
        }) if *modifiers == KeyModifiers::CONTROL
    )
}

thread_local! {
    static SPINNER_ACTION_PANIC: Cell<bool> = const { Cell::new(false) };
    static SPINNER_PANIC_DIAGNOSTIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn install_spinner_panic_hook() {
    INSTALL_PANIC_HOOK.call_once(|| {
        let fallback = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !SPINNER_ACTION_PANIC.with(Cell::get) {
                fallback(info);
                return;
            }

            let message = if let Some(message) = info.payload().downcast_ref::<&str>() {
                *message
            } else if let Some(message) = info.payload().downcast_ref::<String>() {
                message.as_str()
            } else {
                "non-string panic payload"
            };
            let diagnostic = panic_diagnostic(message, &Backtrace::force_capture().to_string());
            SPINNER_PANIC_DIAGNOSTIC.with(|stored| stored.replace(Some(diagnostic)));
        }));
    });
}

fn run_interactive_action<F, T, E, W>(
    action: F,
    context: CancellationToken,
    mut panic_output: W,
) -> ActionCompletion<T, E>
where
    F: FnOnce(CancellationToken) -> Result<T, E>,
    W: Write,
{
    SPINNER_PANIC_DIAGNOSTIC.with(|stored| stored.replace(None));
    SPINNER_ACTION_PANIC.with(|active| active.set(true));
    let completion = catch_unwind(AssertUnwindSafe(|| action(context)));
    SPINNER_ACTION_PANIC.with(|active| active.set(false));
    let diagnostic = SPINNER_PANIC_DIAGNOSTIC.with(|stored| stored.take());
    match completion {
        Ok(output) => ActionCompletion::Output(output),
        Err(_) => {
            if let Some(diagnostic) = diagnostic {
                let _ = panic_output.write_all(diagnostic.as_bytes());
            }
            ActionCompletion::Panicked
        }
    }
}

fn panic_diagnostic(message: &str, stack: &str) -> String {
    let message = message.replace('\n', "\r\n");
    let stack = stack.replace('\n', "\r\n");
    format!("Caught panic:\r\n\r\n{message}\r\n\r\nRestoring terminal...\r\n\r\n{stack}\r\n")
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum TestMode {
    Plain,
    Interactive,
    TimerFailure,
    DriverPanic,
    RenderFailure,
    CancellationBeforeAdmission,
    TerminalSetupFailure,
}

#[cfg(test)]
fn spinner_for_test<F, W, T, E>(
    cancellation: CancellationToken,
    title: impl Into<String>,
    action: F,
    mode: TestMode,
    output: W,
) -> Spinner<T, E>
where
    F: FnOnce(CancellationToken) -> Result<T, E> + Send + 'static,
    W: Write + Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
{
    new_spinner(
        cancellation,
        title.into(),
        action,
        match mode {
            TestMode::Plain => Mode::Plain,
            TestMode::Interactive => Mode::SimulatedTerminal,
            TestMode::TimerFailure => Mode::TimerFailure,
            TestMode::DriverPanic => Mode::DriverPanic,
            TestMode::RenderFailure => Mode::RenderFailure,
            TestMode::CancellationBeforeAdmission => Mode::CancellationBeforeAdmission,
            TestMode::TerminalSetupFailure => Mode::TerminalSetupFailure,
        },
        output,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn block_on_bounded<F>(future: F) -> F::Output
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let (send, receive) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let _ = send.send(crate::runtime::block_on(future));
        });
        receive
            .recv_timeout(Duration::from_secs(2))
            .expect("spinner did not complete within two seconds")
    }

    #[test]
    fn completed_action_returns_its_exact_result() {
        let spinner = spinner_for_test(
            CancellationToken::new(),
            "working",
            |_| Result::<_, &'static str>::Ok(42),
            TestMode::Plain,
            Vec::new(),
        );
        assert!(matches!(crate::runtime::block_on(spinner), Ok(42)));
    }

    #[test]
    fn action_error_is_preserved() {
        let spinner = spinner_for_test(
            CancellationToken::new(),
            "working",
            |_| Result::<(), _>::Err("action failed"),
            TestMode::Plain,
            Vec::new(),
        );
        assert!(matches!(
            crate::runtime::block_on(spinner),
            Err(SpinnerError::Action("action failed"))
        ));
    }

    #[test]
    fn action_environment_does_not_depend_on_terminal_detection() {
        let caller = thread::current().id();
        let plain = spinner_for_test(
            CancellationToken::new(),
            "working",
            move |_| Result::<_, &'static str>::Ok(thread::current().id() == caller),
            TestMode::Plain,
            Vec::new(),
        );
        assert!(matches!(crate::runtime::block_on(plain), Ok(true)));

        let interactive = spinner_for_test(
            CancellationToken::new(),
            "working",
            |_| Result::<_, &'static str>::Ok(thread::current().name().map(str::to_owned)),
            TestMode::Interactive,
            Vec::new(),
        );
        assert!(matches!(
            crate::runtime::block_on(interactive),
            Ok(Some(name)) if name == "ployz-spinner-action"
        ));
    }

    #[test]
    fn plain_mode_ignores_output_failure_and_runs_action() {
        struct BrokenOutput;

        impl Write for BrokenOutput {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let action_calls = calls.clone();
        let spinner = spinner_for_test(
            CancellationToken::new(),
            "working",
            move |_| {
                action_calls.fetch_add(1, Ordering::SeqCst);
                Result::<_, &'static str>::Ok(42)
            },
            TestMode::Plain,
            BrokenOutput,
        );

        assert!(matches!(crate::runtime::block_on(spinner), Ok(42)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn plain_title_write_completes_before_action_invocation() {
        struct OrderingWriter {
            bytes: Arc<Mutex<Vec<u8>>>,
            completed: Arc<AtomicBool>,
        }

        impl Write for OrderingWriter {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                let mut bytes = locked(&self.bytes);
                bytes.extend_from_slice(buffer);
                if bytes.as_slice() == b"working\n" {
                    self.completed.store(true, Ordering::Release);
                }
                Ok(buffer.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let title_written = Arc::new(AtomicBool::new(false));
        let action_observation = title_written.clone();
        let title_bytes = Arc::new(Mutex::new(Vec::new()));
        let spinner = spinner_for_test(
            CancellationToken::new(),
            "working",
            move |_| {
                assert!(action_observation.load(Ordering::Acquire));
                Result::<_, &'static str>::Ok(42)
            },
            TestMode::Plain,
            OrderingWriter {
                bytes: title_bytes.clone(),
                completed: title_written,
            },
        );

        assert!(matches!(block_on_bounded(spinner), Ok(42)));
        assert_eq!(*locked(&title_bytes), b"working\n");
    }

    #[test]
    fn precancelled_interactive_spinner_does_not_invoke_action() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let calls = Arc::new(AtomicUsize::new(0));
        let action_calls = calls.clone();
        let spinner = spinner_for_test(
            cancellation,
            "working",
            move |_| {
                action_calls.fetch_add(1, Ordering::SeqCst);
                Result::<(), &'static str>::Ok(())
            },
            TestMode::Interactive,
            Vec::new(),
        );

        assert!(matches!(
            crate::runtime::block_on(spinner),
            Err(SpinnerError::Cancelled)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cancellation_during_terminal_setup_prevents_action_admission() {
        let calls = Arc::new(AtomicUsize::new(0));
        let action_calls = calls.clone();
        let spinner = spinner_for_test(
            CancellationToken::new(),
            "working",
            move |_| {
                action_calls.fetch_add(1, Ordering::SeqCst);
                Result::<(), &'static str>::Ok(())
            },
            TestMode::CancellationBeforeAdmission,
            Vec::new(),
        );

        assert!(matches!(
            block_on_bounded(spinner),
            Err(SpinnerError::Cancelled)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn terminal_input_setup_failure_returns_error_without_action() {
        let calls = Arc::new(AtomicUsize::new(0));
        let action_calls = calls.clone();
        let spinner = spinner_for_test(
            CancellationToken::new(),
            "working",
            move |_| {
                action_calls.fetch_add(1, Ordering::SeqCst);
                Result::<(), &'static str>::Ok(())
            },
            TestMode::TerminalSetupFailure,
            Vec::new(),
        );

        assert!(matches!(
            block_on_bounded(spinner),
            Err(SpinnerError::Io(error)) if error.to_string().contains("terminal input")
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn completed_spinners_release_reusable_cancellation_waiters() {
        let cancellation = CancellationToken::new();
        for _ in 0..100 {
            let spinner = spinner_for_test(
                cancellation.clone(),
                "working",
                |_| Result::<(), &'static str>::Ok(()),
                TestMode::Interactive,
                Vec::new(),
            );
            assert!(matches!(block_on_bounded(spinner), Ok(())));
            assert_eq!(cancellation.signal.waiter_count(), 0);
        }
    }

    #[test]
    fn external_cancellation_wins_while_context_ignoring_action_continues() {
        let cancellation = CancellationToken::new();
        let action_context = cancellation.clone();
        let (release_action, wait_for_release) = std::sync::mpsc::channel();
        let spinner = spinner_for_test(
            cancellation.clone(),
            "working",
            move |context| {
                assert!(context.ptr_eq(&action_context));
                wait_for_release.recv().unwrap();
                Result::<(), &'static str>::Ok(())
            },
            TestMode::Interactive,
            Vec::new(),
        );
        let interrupt = spinner.interruption.clone();
        let cancel = cancellation.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            cancel.cancel();
        });

        assert!(matches!(
            crate::runtime::block_on(spinner),
            Err(SpinnerError::Cancelled)
        ));
        assert!(!interrupt.is_interrupted());
        release_action.send(()).unwrap();
    }

    #[test]
    fn interruption_returns_while_context_ignoring_action_continues() {
        let (release_action, wait_for_release) = std::sync::mpsc::channel();
        let spinner = spinner_for_test(
            CancellationToken::new(),
            "working",
            move |_| {
                wait_for_release.recv().unwrap();
                Result::<(), &'static str>::Ok(())
            },
            TestMode::Interactive,
            Vec::new(),
        );
        let interrupt = spinner.interruption.clone();
        let send_interrupt = interrupt.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            send_interrupt.interrupt();
        });

        assert!(matches!(
            crate::runtime::block_on(spinner),
            Err(SpinnerError::Interrupted)
        ));
        assert!(interrupt.is_interrupted());
        release_action.send(()).unwrap();
    }

    #[test]
    fn action_result_wins_when_it_arrives_first() {
        let cancellation = CancellationToken::new();
        let spinner = spinner_for_test(
            cancellation.clone(),
            "working",
            |_| Result::<_, &'static str>::Ok(42),
            TestMode::Interactive,
            Vec::new(),
        );

        assert!(matches!(crate::runtime::block_on(spinner), Ok(42)));
        cancellation.cancel();
    }

    #[test]
    fn plain_action_panic_unwinds_and_interactive_panic_is_reported() {
        let plain = spinner_for_test(
            CancellationToken::new(),
            "working",
            |_| -> Result<(), &'static str> { panic!("plain panic sentinel") },
            TestMode::Plain,
            Vec::new(),
        );
        assert!(catch_unwind(AssertUnwindSafe(|| crate::runtime::block_on(plain))).is_err());

        let interactive = spinner_for_test(
            CancellationToken::new(),
            "working",
            |_| -> Result<(), &'static str> { panic!("interactive panic sentinel") },
            TestMode::Interactive,
            Vec::new(),
        );
        assert!(matches!(
            crate::runtime::block_on(interactive),
            Err(SpinnerError::ActionPanicked)
        ));
    }

    #[test]
    fn driver_panic_and_timer_failure_complete_without_hanging() {
        assert_eq!(ACTIVE_TIMER_THREADS.load(Ordering::SeqCst), 0);
        let calls = Arc::new(AtomicUsize::new(0));
        let driver_calls = calls.clone();
        let driver_panic = spinner_for_test(
            CancellationToken::new(),
            "working",
            move |_| -> Result<(), &'static str> {
                driver_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            TestMode::DriverPanic,
            Vec::new(),
        );
        assert!(matches!(
            block_on_bounded(driver_panic),
            Err(SpinnerError::DriverPanicked)
        ));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while ACTIVE_TIMER_THREADS.load(Ordering::SeqCst) != 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "timer thread survived driver panic"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let timer_calls = calls.clone();
        let timer_failure = spinner_for_test(
            CancellationToken::new(),
            "working",
            move |_| -> Result<(), &'static str> {
                timer_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            TestMode::TimerFailure,
            Vec::new(),
        );
        assert!(matches!(
            block_on_bounded(timer_failure),
            Err(SpinnerError::Io(error)) if error.to_string().contains("timer startup")
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn interactive_panic_diagnostic_is_single_crlf_record() {
        let diagnostic = panic_diagnostic("first\nsecond", "frame one\nframe two");
        assert_eq!(
            diagnostic,
            "Caught panic:\r\n\r\nfirst\r\nsecond\r\n\r\nRestoring terminal...\r\n\r\nframe one\r\nframe two\r\n"
        );
        assert!(
            diagnostic
                .as_bytes()
                .windows(2)
                .filter(|pair| pair[1] == b'\n')
                .all(|pair| pair[0] == b'\r')
        );
        assert_eq!(diagnostic.matches("Caught panic:").count(), 1);
        assert_eq!(diagnostic.matches("Restoring terminal...").count(), 1);
    }

    #[test]
    fn interactive_panic_hook_emits_only_for_an_escaping_action_panic() {
        install_spinner_panic_hook();

        let mut caught_output = Vec::new();
        let caught = run_interactive_action(
            |_| {
                let _ = catch_unwind(|| panic!("internally caught sentinel"));
                Result::<_, &'static str>::Ok(42)
            },
            CancellationToken::new(),
            &mut caught_output,
        );
        assert!(matches!(caught, ActionCompletion::Output(Ok(42))));
        assert!(caught_output.is_empty());

        let mut escaping_output = Vec::new();
        let escaping = run_interactive_action(
            |_| -> Result<(), &'static str> { panic!("escaping sentinel") },
            CancellationToken::new(),
            &mut escaping_output,
        );
        assert!(matches!(escaping, ActionCompletion::Panicked));
        let diagnostic = String::from_utf8(escaping_output).unwrap();
        assert_eq!(diagnostic.matches("Caught panic:").count(), 1);
        assert_eq!(diagnostic.matches("escaping sentinel").count(), 1);
        assert_eq!(diagnostic.matches("Restoring terminal...").count(), 1);
        assert!(
            diagnostic
                .as_bytes()
                .windows(2)
                .filter(|pair| pair[1] == b'\n')
                .all(|pair| pair[0] == b'\r')
        );
    }

    #[test]
    fn running_action_does_not_hold_diagnostic_writer_lock() {
        #[derive(Clone)]
        struct SharedWriter(Arc<Mutex<Vec<u8>>>);

        impl Write for SharedWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                locked(&self.0).extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let bytes = Arc::new(Mutex::new(Vec::new()));
        let action_writer = SharedWriter(bytes.clone());
        let renderer_writer = SharedWriter(bytes.clone());
        let (started, await_start) = std::sync::mpsc::channel();
        let (release, await_release) = std::sync::mpsc::channel();
        let action = thread::spawn(move || {
            run_interactive_action(
                move |_| {
                    started.send(()).unwrap();
                    await_release.recv().unwrap();
                    Result::<(), &'static str>::Ok(())
                },
                CancellationToken::new(),
                action_writer,
            )
        });
        await_start.recv_timeout(Duration::from_secs(1)).unwrap();

        let (rendered, await_rendered) = std::sync::mpsc::channel();
        let renderer = thread::spawn(move || {
            let mut writer = renderer_writer;
            writer.write_all(b"frame").unwrap();
            rendered.send(()).unwrap();
        });
        let rendered_while_action_runs =
            await_rendered.recv_timeout(Duration::from_secs(1)).is_ok();
        release.send(()).unwrap();
        renderer.join().unwrap();
        assert!(matches!(
            action.join().unwrap(),
            ActionCompletion::Output(Ok(()))
        ));
        assert!(
            rendered_while_action_runs,
            "renderer blocked on the diagnostic writer while the action ran"
        );
        assert_eq!(*locked(&bytes), b"frame");
    }

    #[test]
    fn render_failure_is_suppressed_until_the_action_finishes() {
        let spinner = spinner_for_test(
            CancellationToken::new(),
            "working",
            |_| {
                thread::sleep(Duration::from_millis(20));
                Result::<_, &'static str>::Ok(42)
            },
            TestMode::RenderFailure,
            Vec::new(),
        );

        assert!(matches!(block_on_bounded(spinner), Ok(42)));
    }

    #[test]
    fn interrupt_classifier_accepts_every_kind_with_exact_control_only() {
        for kind in [
            KeyEventKind::Press,
            KeyEventKind::Repeat,
            KeyEventKind::Release,
        ] {
            let mut key = KeyEvent::new(kind, KeyCode::Char('c'));
            key.modifiers = KeyModifiers::CONTROL;
            assert!(is_interrupt_event(&TerminalEvent::Key(key)));
        }

        for modifiers in [
            KeyModifiers::CONTROL | KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            KeyModifiers::empty(),
        ] {
            let mut key = KeyEvent::new(KeyEventKind::Press, KeyCode::Char('c'));
            key.modifiers = modifiers;
            assert!(!is_interrupt_event(&TerminalEvent::Key(key)));
        }
    }

    #[test]
    fn frame_contract_is_nonempty_and_stable() {
        assert_eq!(FRAMES.len(), 10);
        assert!(FRAMES.iter().all(|frame| !frame.is_empty()));
        assert_eq!(FRAME_INTERVAL, Duration::from_millis(80));
    }
}
