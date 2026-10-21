//! OMP extension-UI ⇄ domain interaction conversions (handoff C5/W04).
//!
//! The domain [`OmpInteraction`] is the seam: wire requests convert into
//! it losslessly (option descriptions/previews, multi-select, deadlines),
//! and responses convert back. The engine's legacy `UserInputQuestion`
//! bridge is one downstream projection of the domain type — labels only —
//! until the W04 UI consumes `OmpInteraction` directly. Nothing is lost
//! before that boundary, and cancellation always wins over a value.

use omp_rpc::wire::{ExtensionUiRequest, ExtensionUiResponse};
use omp_rpc::wire;
use zeron_proto::omp::{
    OmpInteraction, OmpInteractionAnswer, OmpInteractionKind, OmpInteractionOption,
    OmpInteractionQuestion, OmpInteractionResponse, OmpNotifyKind, OmpPresentation,
};
use zeron_proto::{UserInputAnswer, UserInputQuestion};

/// Convert a fire-and-forget presentation request. Dialog methods
/// return `None` (they go through [`to_interaction`]). `open_url` is
/// scheme-validated here: only absolute http/https URLs pass — an
/// extension must never launch `file:`, `javascript:`, app schemes, or
/// bare paths on the host.
pub fn to_presentation(request: &ExtensionUiRequest) -> Option<OmpPresentation> {
    use ExtensionUiRequest as Req;
    Some(match request {
        Req::Notify(req) => OmpPresentation::Notify {
            message: req.message.clone(),
            kind: req.notify_type.map(|kind| match kind {
                wire::NotifyType::Info => OmpNotifyKind::Info,
                wire::NotifyType::Warning => OmpNotifyKind::Warning,
                wire::NotifyType::Error => OmpNotifyKind::Error,
            }),
        },
        Req::SetStatus(req) => OmpPresentation::Status {
            key: req.status_key.clone(),
            text: req.status_text.clone(),
        },
        Req::SetWidget(req) => OmpPresentation::Widget {
            key: req.widget_key.clone(),
            lines: req.widget_lines.clone(),
        },
        Req::SetTitle(req) => OmpPresentation::Title {
            title: req.title.clone(),
        },
        Req::SetEditorText(req) => OmpPresentation::EditorText {
            text: req.text.clone(),
        },
        Req::OpenUrl(req) => {
            if !valid_http_url(&req.url)
                || req.launch_url.as_deref().is_some_and(|url| !valid_http_url(url))
            {
                tracing::warn!("OMP open_url rejected (non-http(s) URL): {}", req.url);
                return None;
            }
            OmpPresentation::OpenUrl {
                url: req.url.clone(),
                launch_url: req.launch_url.clone(),
                instructions: req.instructions.clone(),
            }
        }
        Req::Select(_)
        | Req::Confirm(_)
        | Req::Input(_)
        | Req::Editor(_)
        | Req::Ask(_)
        | Req::Cancel(_) => return None,
    })
}

/// Absolute http/https URL with a non-empty host. Hand-rolled on
/// purpose: this is a scheme allowlist, not a URL parser.
fn valid_http_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !host.is_empty() && !host.chars().any(char::is_whitespace)
}

/// Convert a wire request to the domain interaction. Presentation
/// methods (notify/status/widget/title/editor-text/open_url) are not
/// interactions and return `None`.
pub fn to_interaction(request: &ExtensionUiRequest, generation: u64) -> Option<OmpInteraction> {
    use ExtensionUiRequest as Req;
    let (request_id, title, message, kind, timeout) = match request {
        Req::Select(req) => (
            req.id.clone(),
            req.title.clone(),
            None,
            OmpInteractionKind::Select {
                options: req
                    .options
                    .iter()
                    .enumerate()
                    .map(|(index, label)| OmpInteractionOption {
                        label: label.clone(),
                        description: req
                            .option_details
                            .as_ref()
                            .and_then(|details| details.get(index))
                            .and_then(|detail| detail.description.clone()),
                        preview: None,
                    })
                    .collect(),
            },
            req.timeout,
        ),
        Req::Confirm(req) => (
            req.id.clone(),
            req.title.clone(),
            Some(req.message.clone()),
            OmpInteractionKind::Confirm,
            req.timeout,
        ),
        Req::Input(req) => (
            req.id.clone(),
            req.title.clone(),
            None,
            OmpInteractionKind::Input {
                placeholder: req.placeholder.clone(),
            },
            req.timeout,
        ),
        Req::Editor(req) => (
            req.id.clone(),
            req.title.clone(),
            None,
            OmpInteractionKind::Editor {
                prefill: req.prefill.clone(),
            },
            None,
        ),
        Req::Ask(req) => (
            req.id.clone(),
            String::new(),
            None,
            OmpInteractionKind::Ask {
                questions: req
                    .questions
                    .iter()
                    .map(|question| OmpInteractionQuestion {
                        id: question.id.clone(),
                        header: question.header.clone(),
                        question: question.question.clone(),
                        options: question
                            .options
                            .iter()
                            .map(|option| OmpInteractionOption {
                                label: option.label.clone(),
                                description: option.description.clone(),
                                preview: option.preview.clone(),
                            })
                            .collect(),
                        multi: question.multi,
                        recommended: question
                            .recommended
                            .filter(|index| *index >= 0)
                            .map(|index| index as u32),
                    })
                    .collect(),
            },
            req.timeout,
        ),
        Req::Cancel(req) => (
            req.id.clone(),
            String::new(),
            None,
            OmpInteractionKind::Cancel {
                target_id: req.target_id.clone(),
            },
            None,
        ),
        Req::Notify(_)
        | Req::SetStatus(_)
        | Req::SetWidget(_)
        | Req::SetTitle(_)
        | Req::SetEditorText(_)
        | Req::OpenUrl(_) => return None,
    };
    Some(OmpInteraction {
        request_id,
        generation,
        title,
        message,
        kind,
        timeout_ms: timeout.filter(|ms| *ms > 0).map(|ms| ms as u64),
    })
}

/// Project to the legacy engine question shape (labels only — the known
/// lossy boundary until W04 UI lands). Question ids: `ask` keeps native
/// per-question ids; single-dialog kinds use the request id.
pub fn to_questions(interaction: &OmpInteraction) -> Vec<UserInputQuestion> {
    let single = |options: Vec<String>, prefill: Option<String>, multiline: bool| {
        vec![UserInputQuestion {
            id: interaction.request_id.clone(),
            header: "OMP".into(),
            question: match &interaction.message {
                Some(message) => format!("{}\n{}", interaction.title, message),
                None => interaction.title.clone(),
            },
            options,
            multi_select: false,
            prefill,
            multiline,
        }]
    };
    match &interaction.kind {
        OmpInteractionKind::Select { options } => single(
            options.iter().map(|option| option.label.clone()).collect(),
            None,
            false,
        ),
        OmpInteractionKind::Confirm => single(vec!["Yes".into(), "No".into()], None, false),
        OmpInteractionKind::Input { .. } => single(Vec::new(), None, false),
        OmpInteractionKind::Editor { prefill } => single(Vec::new(), prefill.clone(), true),
        OmpInteractionKind::Ask { questions } => questions
            .iter()
            .map(|question| UserInputQuestion {
                id: question.id.clone(),
                header: question.header.clone().unwrap_or_else(|| "OMP".into()),
                question: question.question.clone(),
                options: question
                    .options
                    .iter()
                    .map(|option| option.label.clone())
                    .collect(),
                multi_select: question.multi,
                prefill: None,
                multiline: false,
            })
            .collect(),
        OmpInteractionKind::Cancel { .. } => Vec::new(),
    }
}

/// Build the domain response from legacy answers. A value that is not a
/// known option (select) or not Yes/No (confirm) is a cancellation, never
/// a guess — approvals fail closed.
pub fn from_answers(
    interaction: &OmpInteraction,
    answers: &[UserInputAnswer],
) -> OmpInteractionResponse {
    let labels = |question_id: &str| -> Vec<String> {
        answers
            .iter()
            .find(|answer| answer.question_id == question_id)
            .map(|answer| answer.labels.clone())
            .unwrap_or_default()
    };
    let first = |question_id: &str| labels(question_id).into_iter().next();
    match &interaction.kind {
        OmpInteractionKind::Select { options } => {
            match first(&interaction.request_id)
                .filter(|value| options.iter().any(|option| &option.label == value))
            {
                Some(value) => OmpInteractionResponse::Value { value },
                None => OmpInteractionResponse::Cancelled { timed_out: false },
            }
        }
        OmpInteractionKind::Confirm => match first(&interaction.request_id).as_deref() {
            Some("Yes") => OmpInteractionResponse::Confirmed { confirmed: true },
            Some("No") => OmpInteractionResponse::Confirmed { confirmed: false },
            _ => OmpInteractionResponse::Cancelled { timed_out: false },
        },
        OmpInteractionKind::Input { .. } | OmpInteractionKind::Editor { .. } => {
            match first(&interaction.request_id) {
                Some(value) => OmpInteractionResponse::Value { value },
                None => OmpInteractionResponse::Cancelled { timed_out: false },
            }
        }
        OmpInteractionKind::Ask { questions } => OmpInteractionResponse::Answers {
            answers: questions
                .iter()
                .map(|question| {
                    let given = labels(&question.id);
                    let selected: Vec<String> = given
                        .iter()
                        .filter(|label| {
                            question.options.iter().any(|option| &option.label == *label)
                        })
                        .cloned()
                        .collect();
                    let custom = given
                        .iter()
                        .find(|label| !selected.iter().any(|picked| picked == *label))
                        .cloned();
                    OmpInteractionAnswer {
                        question_id: question.id.clone(),
                        selected_options: selected,
                        custom_input: custom,
                    }
                })
                .collect(),
        },
        OmpInteractionKind::Cancel { .. } => {
            OmpInteractionResponse::Cancelled { timed_out: false }
        }
    }
}

/// Convert the domain response back to the wire reply for `request_id`.
pub fn to_wire_response(
    request_id: &str,
    response: OmpInteractionResponse,
) -> ExtensionUiResponse {
    match response {
        OmpInteractionResponse::Value { value } => {
            ExtensionUiResponse::ValueUiResponse(wire::ValueUiResponse {
                id: request_id.to_owned(),
                value,
            })
        }
        OmpInteractionResponse::Confirmed { confirmed } => {
            ExtensionUiResponse::ConfirmUiResponse(wire::ConfirmUiResponse {
                id: request_id.to_owned(),
                confirmed,
            })
        }
        OmpInteractionResponse::Answers { answers } => {
            ExtensionUiResponse::AnswersUiResponse(wire::AnswersUiResponse {
                id: request_id.to_owned(),
                answers: answers
                    .into_iter()
                    .map(|answer| wire::AskAnswer {
                        id: answer.question_id,
                        selected_options: answer.selected_options,
                        custom_input: answer.custom_input,
                    })
                    .collect(),
            })
        }
        OmpInteractionResponse::Cancelled { timed_out } => {
            ExtensionUiResponse::CancelUiResponse(wire::CancelUiResponse {
                id: request_id.to_owned(),
                cancelled: wire::LitTrue,
                timed_out: timed_out.then_some(true),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omp_rpc::wire::{AskOption, AskQuestion, AskUiRequest, ConfirmUiRequest, SelectUiRequest};

    #[test]
    fn ask_round_trips_descriptions_multi_and_custom_input() {
        let wire_req = ExtensionUiRequest::Ask(AskUiRequest {
            id: "r9".into(),
            questions: vec![AskQuestion {
                id: "q1".into(),
                question: "Which?".into(),
                options: vec![AskOption {
                    label: "Alpha".into(),
                    description: Some("first option".into()),
                    preview: Some("preview text".into()),
                }],
                header: Some("Pick".into()),
                multi: true,
                recommended: Some(0),
            }],
            timeout: Some(5_000),
        });
        let interaction = to_interaction(&wire_req, 4).unwrap();
        assert_eq!(interaction.generation, 4);
        assert_eq!(interaction.timeout_ms, Some(5_000));
        let OmpInteractionKind::Ask { questions } = &interaction.kind else {
            panic!("expected ask");
        };
        assert_eq!(questions[0].options[0].description.as_deref(), Some("first option"));
        assert_eq!(questions[0].options[0].preview.as_deref(), Some("preview text"));
        assert!(questions[0].multi);

        let questions = to_questions(&interaction);
        assert_eq!(questions[0].id, "q1");
        let response = from_answers(
            &interaction,
            &[UserInputAnswer {
                question_id: "q1".into(),
                labels: vec!["Alpha".into(), "my own words".into()],
            }],
        );
        let OmpInteractionResponse::Answers { answers } = &response else {
            panic!("expected answers");
        };
        assert_eq!(answers[0].selected_options, ["Alpha"]);
        assert_eq!(answers[0].custom_input.as_deref(), Some("my own words"));
        let wire_back = to_wire_response("r9", response);
        assert!(matches!(wire_back, ExtensionUiResponse::AnswersUiResponse(_)));
    }

    #[test]
    fn confirm_and_select_fail_closed_on_unknown_answers() {
        let confirm = to_interaction(
            &ExtensionUiRequest::Confirm(ConfirmUiRequest {
                id: "c1".into(),
                title: "Run?".into(),
                message: "bash rm".into(),
                timeout: None,
            }),
            1,
        )
        .unwrap();
        let bogus = from_answers(
            &confirm,
            &[UserInputAnswer {
                question_id: "c1".into(),
                labels: vec!["Maybe".into()],
            }],
        );
        assert!(matches!(bogus, OmpInteractionResponse::Cancelled { .. }));

        let select = to_interaction(
            &ExtensionUiRequest::Select(SelectUiRequest {
                id: "s1".into(),
                title: "Pick".into(),
                options: vec!["a".into()],
                option_details: None,
                timeout: None,
            }),
            1,
        )
        .unwrap();
        let bogus = from_answers(
            &select,
            &[UserInputAnswer {
                question_id: "s1".into(),
                labels: vec!["not-an-option".into()],
            }],
        );
        assert!(matches!(bogus, OmpInteractionResponse::Cancelled { .. }));
    }

    #[test]
    fn presentation_converts_and_open_url_is_scheme_gated() {
        use omp_rpc::wire::{NotifyUiRequest, OpenUrlUiRequest, SetStatusUiRequest};
        let notify = to_presentation(&ExtensionUiRequest::Notify(NotifyUiRequest {
            id: "n1".into(),
            message: "saved".into(),
            notify_type: Some(wire::NotifyType::Warning),
        }));
        assert!(matches!(
            notify,
            Some(OmpPresentation::Notify { kind: Some(OmpNotifyKind::Warning), .. })
        ));
        let cleared = to_presentation(&ExtensionUiRequest::SetStatus(SetStatusUiRequest {
            id: "s1".into(),
            status_key: "build".into(),
            status_text: None,
        }));
        assert!(matches!(
            cleared,
            Some(OmpPresentation::Status { text: None, .. })
        ));
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "omp://x", "/tmp/x", "https://"] {
            let rejected = to_presentation(&ExtensionUiRequest::OpenUrl(OpenUrlUiRequest {
                id: "u1".into(),
                url: bad.into(),
                launch_url: None,
                instructions: None,
            }));
            assert!(rejected.is_none(), "{bad} must be rejected");
        }
        let ok = to_presentation(&ExtensionUiRequest::OpenUrl(OpenUrlUiRequest {
            id: "u2".into(),
            url: "https://example.com/oauth?x=1".into(),
            launch_url: Some("http://127.0.0.1:9911/r".into()),
            instructions: None,
        }));
        assert!(matches!(ok, Some(OmpPresentation::OpenUrl { .. })));
    }
}
