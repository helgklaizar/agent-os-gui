//! `agent-watch` (a core module, not a separate crate: a new workspace member breaks the GUI layout test):
//! a model-based observer behind the `Observer` interface. Core rules are the first line, the model
//! `claude-haiku-5-5` through the core provider is the second. An unavailable model or one that answers garbage means an unavailable
//! observer, and therefore closed dangerous actions (fail-closed). The model only asks for confirmation: blocking
//! without a question stays with the rules.

mod classifier;
mod observer;
mod remote;
mod verdict;

pub use classifier::{
    AccountPicker, Classifier, ClassifyError, DEFAULT_MODEL, DEFAULT_TIMEOUT, MAX_SUBJECT_CHARS,
    ModelClassifier, StoredAccounts, Subject,
};
pub use observer::ModelObserver;
pub use remote::RemoteObserver;
pub use verdict::{Class, Verdict, parse_verdict};
