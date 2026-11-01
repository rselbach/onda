//! Optional transcript cleanup through OpenRouter (05, "Cleanup").
//!
//! Everything here is pure: the prompt, the model catalog, request building,
//! response parsing, guardrails, the attempt/retry decisions, notice copy,
//! and the settings page's API key save rules. The HTTP client and the
//! credential store live outside the domain.

mod api_key_field;
mod catalog;
mod guardrails;
mod openrouter;
mod outcome;
mod prompt;

pub use api_key_field::{KeyBaseline, KeyWrite, key_write};
pub use catalog::{
    CLEANUP_MODELS, CleanupModel, ReasoningEffort, default_model, model_for_slug, resolve_model,
};
pub use guardrails::{GuardrailViolation, check_guardrails};
pub use openrouter::{
    ApiKey, CleanupError, CleanupRequest, ENDPOINT, REQUEST_TIMEOUT, RequestBody, build_request,
    parse_response, user_message,
};
pub use outcome::{
    CleanupNotice, CleanupOutcome, CleanupStart, CleanupStep, TEST_FAILED_TITLE, TEST_SAMPLE,
    UNAVAILABLE_ERROR, after_first_attempt, after_retry, start, test_result_line, test_said_line,
};
pub use prompt::SYSTEM_PROMPT;
