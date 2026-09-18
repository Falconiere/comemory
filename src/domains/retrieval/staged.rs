//! The pause point between a deterministic ranking and its learned order (#213).
//!
//! A command core that may call an out-of-process scorer cannot simply do it
//! inline: `comemory serve` runs every command under one shared
//! `Mutex<Connection>`, so a slow model inside that closure stalls unrelated
//! requests. A core therefore returns a [`Staged`] value — either the finished
//! result, or a [`Paused`] one whose scoring call borrows nothing and whose
//! remainder takes the connection back afterwards.
//!
//! The CLI owns its connection outright and resolves both arms in place with
//! [`resolve`]. `serve::routes::staged` runs the three phases in three blocking
//! tasks and holds the guard only for the first and the third.

use crate::domains::retrieval::learned_rerank::LearnedCall;
use crate::prelude::*;
use crate::utilities::context::Ctx;
use crate::utilities::rerank_outcome::RerankOutcome;

/// The remainder of a paused command core: pagination, telemetry, and whatever
/// assembly the surface does after its order is final.
///
/// Boxed rather than a generic parameter so a caller needs one type parameter
/// (the result) instead of two, and `Send + 'static` because it crosses a
/// blocking-task boundary — which is what forbids it capturing a borrow.
pub struct FinishStep<T>(Continuation<T>);

/// The boxed continuation behind [`FinishStep`], named so the signature stays
/// readable and `clippy::type_complexity` has nothing to object to.
type Continuation<T> =
    Box<dyn for<'a, 'b> FnOnce(&'b mut Ctx<'a>, RerankOutcome) -> Result<T> + Send>;

impl<T> FinishStep<T> {
    /// Wrap a surface's phase-three continuation.
    pub fn new(
        f: impl for<'a, 'b> FnOnce(&'b mut Ctx<'a>, RerankOutcome) -> Result<T> + Send + 'static,
    ) -> Self {
        Self(Box::new(f))
    }

    /// Run it, with the connection available again.
    pub fn run(self, ctx: &mut Ctx<'_>, outcome: RerankOutcome) -> Result<T> {
        (self.0)(ctx, outcome)
    }
}

/// A command core paused for inference. Nothing here borrows a connection.
pub struct Paused<T> {
    /// Boxed so the `Paused` arm of [`Staged`] stays the same size as the
    /// `Ready` one — a scoring call carries a program, its arguments and the
    /// whole request.
    call: Box<LearnedCall>,
    finish: FinishStep<T>,
}

impl<T> Paused<T> {
    /// Pair a scoring call with the step that consumes its outcome.
    pub fn new(call: LearnedCall, finish: FinishStep<T>) -> Self {
        Self {
            call: Box::new(call),
            finish,
        }
    }

    /// Split into the connection-free call and the step that needs the
    /// connection back — what an adapter holding a shared lock uses to drop it.
    pub fn into_parts(self) -> (LearnedCall, FinishStep<T>) {
        (*self.call, self.finish)
    }
}

/// A command core's outcome: finished, or waiting on a scorer.
pub enum Staged<T> {
    /// No learned ordering stage ran; the result is final.
    Ready(T),
    /// Inference is pending.
    Paused(Paused<T>),
}

impl<T: 'static> Staged<T> {
    /// Project the eventual result on either arm. The `Ready` arm applies `f`
    /// at once; the `Paused` arm defers it to phase three, so a failure there
    /// surfaces after inference rather than before it.
    pub fn map<U: 'static>(
        self,
        f: impl FnOnce(T) -> Result<U> + Send + 'static,
    ) -> Result<Staged<U>> {
        match self {
            Staged::Ready(value) => f(value).map(Staged::Ready),
            Staged::Paused(paused) => {
                let (call, finish) = paused.into_parts();
                let composed = FinishStep::new(move |ctx, outcome| f(finish.run(ctx, outcome)?));
                Ok(Staged::Paused(Paused::new(call, composed)))
            }
        }
    }
}

/// Run a staged core to completion on one connection.
///
/// The CLI's path: it owns `comemory.db` for the length of the command, so
/// there is no shared lock to release and the scorer runs in place.
pub fn resolve<T>(ctx: &mut Ctx<'_>, staged: Staged<T>) -> Result<T> {
    match staged {
        Staged::Ready(value) => Ok(value),
        Staged::Paused(paused) => {
            let (call, finish) = paused.into_parts();
            let outcome = call.score();
            finish.run(ctx, outcome)
        }
    }
}

#[cfg(test)]
#[path = "tests/staged.rs"]
mod tests;
