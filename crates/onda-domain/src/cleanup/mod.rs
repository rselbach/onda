//! Optional transcript cleanup through a cloud provider (OpenRouter or
//! Fireworks AI).
//!
//! Everything here is pure: the prompt, the model catalog, request building,
//! response parsing, guardrails, the attempt/retry decisions, notice copy,
//! and the settings page's API key save rules. The HTTP client and the
//! credential store live outside the domain.

mod api_key_field;
mod catalog;
mod fireworks;
mod guardrails;
mod listing;
mod openrouter;
mod outcome;
mod prompt;
mod provider;

pub use api_key_field::{KeyBaseline, KeyWrite, key_write};
pub use catalog::{
    CLEANUP_MODELS, CleanupModel, ReasoningEffort, custom_model_slug, default_model,
    model_for_slug, resolve_model,
};
pub use fireworks::FIREWORKS_MODELS;
pub use guardrails::{GuardrailViolation, check_guardrails};
pub use listing::{
    MAX_MODEL_PAGES, ModelsPage, ModelsRequest, finish_models, models_request, parse_models_page,
};
pub use openrouter::{
    ApiKey, CleanupError, CleanupRequest, ENDPOINT, MODELS_ENDPOINT, REQUEST_TIMEOUT, RequestBody,
    build_request, parse_response, user_message,
};
pub use outcome::{
    CleanupNotice, CleanupOutcome, CleanupStart, CleanupStep, TEST_FAILED_TITLE, TEST_SAMPLE,
    after_first_attempt, after_retry, start, test_result_line, test_said_line, unavailable_error,
};
pub use prompt::SYSTEM_PROMPT;
pub use provider::CleanupProvider;
