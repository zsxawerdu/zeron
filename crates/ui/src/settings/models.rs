//! Settings → Models (W05): the OMP model/role hub plus onboarding.
//!
//! Three layers stay visibly separate here, because OMP keeps them
//! separate:
//! - **Onboarding/accounts**: executable/version readiness and
//!   OMP-managed provider sign-in (`OmpLogin*` flows). Zeron never
//!   touches OMP's credential store; logout goes through OMP.
//! - **Persistent roles**: role assignments, cycle order, fallback
//!   chains and the default thinking level, written through
//!   `omp config`. Assigning a role never changes a live session.
//! - **Session-only switching**: the composer picker's job. This
//!   page deliberately has no "apply to session" action, so the two
//!   can never be confused.
//!
//! The catalog is OMP-discovered (`OmpModelDescriptor`, unknown
//! native metadata preserved). When live discovery fails, the
//! engine answers the last-known scoped catalog with `stale: true`;
//! this page labels that instead of presenting it as live, and an
//! unavailable catalog shows its error rather than a static list.

use gpui::{
    AnyElement, Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px,
};

use zeron_proto::omp::{
    OmpCatalogSnapshot, OmpLoginPoll, OmpLoginStatus, OmpLogoutAccount, OmpModelDescriptor,
    OmpModelRoles, OmpProviderLimits, OmpReadiness,
};
use zeron_proto::{UserInputAnswer, UserInputQuestion};
use zeron_rpc::methods;

use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::popover::Loadable;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

/// Models visible under the current search + provider filter. Pure.
pub fn visible_models<'a>(
    models: &'a [OmpModelDescriptor],
    provider: Option<&str>,
    query: &str,
) -> Vec<&'a OmpModelDescriptor> {
    let query = query.trim().to_lowercase();
    models
        .iter()
        .filter(|model| provider.is_none_or(|p| model.model.provider == p))
        .filter(|model| {
            query.is_empty()
                || model.label.to_lowercase().contains(&query)
                || model.model.to_string().to_lowercase().contains(&query)
        })
        .collect()
}

/// Providers in catalog order with their model counts. Pure.
pub fn provider_counts(models: &[OmpModelDescriptor]) -> Vec<(String, usize)> {
    let mut providers: Vec<(String, usize)> = Vec::new();
    for model in models {
        match providers.iter_mut().find(|(p, _)| p == &model.model.provider) {
            Some((_, count)) => *count += 1,
            None => providers.push((model.model.provider.clone(), 1)),
        }
    }
    providers
}

/// Compact context-window label (`272k`, `1M`). Pure.
pub fn context_label(tokens: Option<i64>) -> String {
    match tokens {
        None => "—".into(),
        Some(n) if n >= 1_000_000 => format!("{}M", n / 1_000_000),
        Some(n) if n >= 1_000 => format!("{}k", n / 1_000),
        Some(n) => n.to_string(),
    }
}

struct LoginView {
    poll: OmpLoginPoll,
    opened_url: Option<String>,
    /// question id → selected labels for the pending questions.
    picks: std::collections::HashMap<String, Vec<String>>,
}

pub struct ModelsPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    readiness: Loadable<OmpReadiness>,
    catalog: Loadable<OmpCatalogSnapshot>,
    roles: Loadable<OmpModelRoles>,
    limits: Loadable<OmpProviderLimits>,
    accounts: std::collections::HashMap<String, Loadable<Vec<OmpLogoutAccount>>>,
    search: Entity<ComposerInput>,
    provider_filter: Option<String>,
    selected: Option<String>,
    /// Role being assigned: the next model-row click persists it.
    assigning_role: Option<String>,
    login: Option<LoginView>,
    login_text: Entity<ComposerInput>,
    error: Option<SharedString>,
    busy: bool,
    task: Option<Task<()>>,
    login_task: Option<Task<()>>,
    answer_task: Option<Task<()>>,
    _observe: Subscription,
    _search_sub: Subscription,
}

impl ModelsPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&state, |_, _, cx| cx.notify());
        let search = cx.new(|cx| ComposerInput::new("Search models…", cx));
        let search_sub = cx.subscribe(&search, |_, _, event, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                cx.notify();
            }
        });
        let login_text = cx.new(|cx| ComposerInput::new("Type your answer…", cx));
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            readiness: Loadable::Idle,
            catalog: Loadable::Idle,
            roles: Loadable::Idle,
            limits: Loadable::Idle,
            accounts: std::collections::HashMap::new(),
            search,
            provider_filter: None,
            selected: None,
            assigning_role: None,
            login: None,
            login_text,
            error: None,
            busy: false,
            task: None,
            login_task: None,
            answer_task: None,
            _observe: observe,
            _search_sub: search_sub,
        };
        page.load_all(cx);
        page
    }

    fn engine(&self, cx: &mut Context<Self>) -> Option<crate::state::EngineHandle> {
        self.state.read(cx).engine().cloned()
    }

    /// Load readiness, catalog, roles and limits in one round of
    /// independent RPCs; each surface settles on its own.
    fn load_all(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine(cx) else {
            self.readiness = Loadable::Error("Engine not connected".into());
            return;
        };
        self.readiness = Loadable::Loading;
        self.catalog = Loadable::Loading;
        self.roles = Loadable::Loading;
        self.limits = Loadable::Loading;
        self.task = Some(cx.spawn(async move |this, cx| {
            let readiness = engine.client().call(methods::OMP_READINESS, serde_json::json!({})).await;
            let catalog = engine.client().call(methods::OMP_MODELS, serde_json::json!({})).await;
            let roles = engine.client().call(methods::OMP_MODEL_ROLES, serde_json::json!({})).await;
            let limits = engine.client().call(methods::OMP_PROVIDER_LIMITS, serde_json::json!({})).await;
            this.update(cx, |page, cx| {
                page.readiness = settle(readiness);
                page.catalog = settle(catalog);
                page.roles = settle(roles);
                page.limits = settle(limits);
                if page.selected.is_none()
                    && let Some(snapshot) = page.catalog.ready()
                    && let Some(first) = snapshot.models.first()
                {
                    page.selected = Some(first.model.to_string());
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn call_roles(
        &mut self,
        method: &'static str,
        params: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.engine(cx) else {
            return;
        };
        self.busy = true;
        self.error = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = engine.client().call(method, params).await;
            this.update(cx, |page, cx| {
                page.busy = false;
                match result.and_then(|v| {
                    serde_json::from_value::<OmpModelRoles>(v).map_err(|e| zeron_rpc::RpcError::Failed(e.to_string()))
                }) {
                    Ok(roles) => page.roles = Loadable::Ready(roles),
                    Err(err) => page.error = Some(err.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn assign_role(&mut self, role: String, selector: String, cx: &mut Context<Self>) {
        self.assigning_role = None;
        self.call_roles(
            methods::OMP_SET_MODEL_ROLE,
            serde_json::json!({ "role": role, "selector": selector }),
            cx,
        );
    }

    fn clear_role(&mut self, role: String, cx: &mut Context<Self>) {
        self.call_roles(
            methods::OMP_SET_MODEL_ROLE,
            serde_json::json!({ "role": role }),
            cx,
        );
    }

    fn start_login(&mut self, provider_id: String, cx: &mut Context<Self>) {
        let Some(engine) = self.engine(cx) else {
            return;
        };
        self.error = None;
        self.login_task = Some(cx.spawn(async move |this, cx| {
            let started = engine
                .client()
                .call(
                    methods::OMP_LOGIN_START,
                    serde_json::json!({ "providerId": provider_id }),
                )
                .await;
            let login_id = started
                .ok()
                .and_then(|v| v.get("loginId").and_then(|id| id.as_str()).map(str::to_owned));
            let Some(login_id) = login_id else {
                this.update(cx, |page, cx| {
                    page.error = Some("Could not start the OMP login".into());
                    cx.notify();
                })
                .ok();
                return;
            };
            // Poll until the flow settles; each poll repaints.
            loop {
                let polled = engine
                    .client()
                    .call(
                        methods::OMP_LOGIN_POLL,
                        serde_json::json!({ "loginId": login_id }),
                    )
                    .await
                    .and_then(|v| serde_json::from_value::<OmpLoginPoll>(v).map_err(|e| zeron_rpc::RpcError::Failed(e.to_string())));
                let settled = match &polled {
                    Ok(poll) => {
                        let mut settled = matches!(
                            poll.status,
                            OmpLoginStatus::Done | OmpLoginStatus::Failed | OmpLoginStatus::Cancelled
                        );
                        this.update(cx, |page, cx| {
                            match &polled {
                                Ok(poll) => {
                                    if let Some(view) = &mut page.login {
                                        view.poll = poll.clone();
                                    } else {
                                        page.login = Some(LoginView {
                                            poll: poll.clone(),
                                            opened_url: None,
                                            picks: Default::default(),
                                        });
                                    }
                                    if let Some(view) = &mut page.login
                                        && let Some(url) = &view.poll.open_url
                                        && view.opened_url.as_deref() != Some(url.as_str())
                                    {
                                        view.opened_url = Some(url.clone());
                                        cx.open_url(url);
                                    }
                                }
                                Err(err) => {
                                    page.error = Some(err.to_string().into());
                                    settled = true;
                                }
                            }
                            cx.notify();
                        })
                        .ok();
                        settled
                    }
                    Err(_) => true,
                };
                if settled {
                    break;
                }
                // GPUI tasks have no Tokio reactor (see composer).
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(700))
                    .await;
            }
            this.update(cx, |page, cx| {
                if matches!(
                    page.login.as_ref().map(|v| v.poll.status),
                    Some(OmpLoginStatus::Done)
                ) {
                    page.login = None;
                    page.load_all(cx);
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn answer_login(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine(cx) else {
            return;
        };
        let Some(view) = &self.login else {
            return;
        };
        let login_id = view.poll.login_id.clone();
        let text = self.login_text.read(cx).text().trim().to_string();
        let answers: Vec<UserInputAnswer> = view
            .poll
            .questions
            .iter()
            .map(|question| {
                let mut labels = view.picks.get(&question.id).cloned().unwrap_or_default();
                if labels.is_empty() && !text.is_empty() && question.options.is_empty() {
                    labels.push(text.clone());
                }
                UserInputAnswer {
                    question_id: question.id.clone(),
                    labels,
                }
            })
            .collect();
        self.answer_task = Some(cx.spawn(async move |this, cx| {
            let _ = engine
                .client()
                .call(
                    methods::OMP_LOGIN_ANSWER,
                    serde_json::json!({ "loginId": login_id, "answers": answers }),
                )
                .await;
            this.update(cx, |page, cx| {
                if let Some(view) = &mut page.login {
                    view.picks.clear();
                }
                page.login_text.update(cx, |input, cx| input.set_text("", cx));
                cx.notify();
            })
            .ok();
            // The start_login poll loop is still running and picks up
            // the flow's next stage on its next iteration.
        }));
        cx.notify();
    }

    fn cancel_login(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.engine(cx) else {
            return;
        };
        let Some(login_id) = self.login.as_ref().map(|v| v.poll.login_id.clone()) else {
            return;
        };
        self.login = None;
        self.login_task = Some(cx.spawn(async move |this, cx| {
            let _ = engine
                .client()
                .call(
                    methods::OMP_LOGIN_CANCEL,
                    serde_json::json!({ "loginId": login_id }),
                )
                .await;
            this.update(cx, |page, cx| {
                page.load_all(cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn toggle_accounts(&mut self, provider_id: String, cx: &mut Context<Self>) {
        if self.accounts.remove(&provider_id).is_some() {
            cx.notify();
            return;
        }
        let Some(engine) = self.engine(cx) else {
            return;
        };
        self.accounts
            .insert(provider_id.clone(), Loadable::Loading);
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(
                    methods::OMP_LOGOUT_ACCOUNTS,
                    serde_json::json!({ "providerId": provider_id }),
                )
                .await;
            this.update(cx, |page, cx| {
                page.accounts.insert(provider_id, settle(result));
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn logout(&mut self, account: OmpLogoutAccount, cx: &mut Context<Self>) {
        let Some(engine) = self.engine(cx) else {
            return;
        };
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(
                    methods::OMP_LOGOUT,
                    serde_json::json!({
                        "providerId": account.provider,
                        "credentialId": account.credential_id,
                    }),
                )
                .await;
            this.update(cx, |page, cx| {
                page.busy = false;
                if let Err(err) = result {
                    page.error = Some(err.to_string().into());
                }
                page.accounts.remove(&account.provider);
                page.load_all(cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
}

/// Roles currently assigned to one model selector, comma-joined
/// for a list-row badge. Pure.
pub fn roles_for_model(roles: &Loadable<OmpModelRoles>, selector: &str) -> String {
    match roles.ready() {
        Some(roles) => roles
            .assignments
            .iter()
            .filter(|(_, assigned)| assigned.as_str() == selector)
            .map(|(role, _)| role.clone())
            .collect::<Vec<_>>()
            .join(", "),
        None => String::new(),
    }
}

fn small_button(theme: &Theme, id: impl Into<SharedString>, label: impl Into<SharedString>) -> gpui::Stateful<gpui::Div> {
    let _ = theme;
    div()
        .id(id.into())
        .flex_none()
        .px(px(10.0))
        .py(px(4.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border)
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(|s| s.bg(theme.surface_raised).text_color(theme.text))
        .tab_index(0)
        .role(gpui::Role::Button)
        .child(label.into())
}

fn settle<T: serde::de::DeserializeOwned>(
    result: Result<serde_json::Value, zeron_rpc::RpcError>,
) -> Loadable<T> {
    match result {
        Ok(value) => match serde_json::from_value::<T>(value) {
            Ok(parsed) => Loadable::Ready(parsed),
            Err(err) => Loadable::Error(err.to_string()),
        },
        Err(err) => Loadable::Error(err.to_string()),
    }
}

impl ModelsPage {
    fn render_onboarding(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut block = widgets::section_card(theme).flex().flex_col().gap(px(6.0));
        match &self.readiness {
            Loadable::Idle | Loadable::Loading => {
                block = block.child(div().text_color(theme.text_muted).child("Checking OMP…"));
            }
            Loadable::Error(error) => {
                block = block.child(
                    div()
                        .text_color(theme.danger)
                        .child(SharedString::from(error.clone())),
                );
            }
            Loadable::Ready(readiness) => {
                if let Some(error) = &readiness.discovery_error {
                    // Missing/unusable executable: the actionable
                    // cause verbatim. Installation is OMP's own —
                    // Zeron deliberately offers no installer for it.
                    block = block
                        .child(
                            div()
                                .text_color(theme.danger)
                                .child(SharedString::from(error.clone())),
                        )
                        .child(
                            div().text_color(theme.text_muted).child(
                                "Install or update OMP with its own installer, then refresh. \
                                 Zeron does not install or update the OMP runtime.",
                            ),
                        );
                } else {
                    let version = readiness.version.as_deref().unwrap_or("unknown");
                    block = block.child(
                        div().flex().flex_row().gap(px(8.0)).children([
                            widgets::badge(
                                theme,
                                SharedString::from(format!("omp {version}")),
                            ),
                            if readiness.qualified {
                                widgets::badge_active(theme, "qualified build")
                            } else {
                                widgets::badge(theme, "newer than the qualified build")
                            },
                            if readiness.ready_for_first_turn() {
                                widgets::badge_active(theme, "ready for a first session")
                            } else {
                                widgets::badge(theme, "sign in to start a first session")
                            },
                        ]),
                    );
                }
                for provider in &readiness.providers {
                    let provider_id = provider.id.clone();
                    let expanded = self.accounts.contains_key(&provider.id);
                    let mut row = div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(div().flex_1().child(SharedString::from(provider.name.clone())))
                        .child(if provider.authenticated {
                            widgets::badge_active(theme, "signed in")
                        } else if provider.available {
                            widgets::badge(theme, "not signed in")
                        } else {
                            widgets::badge(theme, "unavailable")
                        });
                    if provider.authenticated {
                        row = row.child(
                            small_button(
                                theme,
                                SharedString::from(format!("omp-accounts-{provider_id}")),
                                if expanded { "Hide accounts" } else { "Accounts" },
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_accounts(provider_id.clone(), cx);
                            })),
                        );
                    } else if provider.available {
                        let login_provider = provider.id.clone();
                        row = row.child(
                            small_button(
                                theme,
                                SharedString::from(format!("omp-login-{provider_id}")),
                                "Sign in",
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.start_login(login_provider.clone(), cx);
                            })),
                        );
                    }
                    block = block.child(row);
                    if expanded {
                        match self.accounts.get(&provider.id) {
                            Some(Loadable::Ready(accounts)) if accounts.is_empty() => {
                                block = block.child(
                                    div()
                                        .text_color(theme.text_muted)
                                        .child("No stored credentials OMP can remove."),
                                );
                            }
                            Some(Loadable::Ready(accounts)) => {
                                for account in accounts.clone() {
                                    let label = format!(
                                        "{} · {} · {}{}",
                                        account.label,
                                        account.detail,
                                        account.kind,
                                        if account.active { " · active" } else { "" }
                                    );
                                    block = block.child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(8.0))
                                            .child(div().flex_1().child(label))
                                            .child(
                                                small_button(
                                                    theme,
                                                    SharedString::from(format!(
                                                        "omp-logout-{}",
                                                        account.credential_id
                                                    )),
                                                    "Sign out",
                                                )
                                                .on_click(cx.listener(
                                                    move |this, _, _, cx| {
                                                        this.logout(account.clone(), cx);
                                                    },
                                                )),
                                            ),
                                    );
                                }
                            }
                            Some(Loadable::Error(error)) => {
                                block = block.child(
                                    div()
                                        .text_color(theme.danger)
                                        .child(SharedString::from(error.clone())),
                                );
                            }
                            _ => {
                                block = block.child(
                                    div().text_color(theme.text_muted).child("Loading accounts…"),
                                );
                            }
                        }
                    }
                }
            }
        }
        if let Some(view) = &self.login {
            let poll = view.poll.clone();
            let mut panel = widgets::section_card(theme)
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div().child(SharedString::from(format!(
                        "Signing in to {}…",
                        poll.provider_id
                    ))),
                );
            match poll.status {
                OmpLoginStatus::Running => {
                    panel = panel.child(
                        div()
                            .text_color(theme.text_muted)
                            .child("Waiting for OMP — finish the sign-in in your browser."),
                    );
                }
                OmpLoginStatus::AwaitingInput => {
                    for question in &poll.questions {
                        panel = panel.child(self.render_login_question(question, theme, cx));
                    }
                    panel = panel.child(
                        small_button(theme, "omp-login-submit", "Submit")
                            .on_click(cx.listener(|this, _, _, cx| this.answer_login(cx))),
                    );
                }
                OmpLoginStatus::Failed => {
                    panel = panel.child(
                        div()
                            .text_color(theme.danger)
                            .child(SharedString::from(poll.error.clone().unwrap_or_default())),
                    );
                }
                OmpLoginStatus::Done | OmpLoginStatus::Cancelled => {}
            }
            panel = panel.child(
                small_button(theme, "omp-login-cancel", "Cancel sign-in")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_login(cx))),
            );
            block = block.child(panel);
        }
        widgets::section(theme, "Onboarding & accounts", block).into_any_element()
    }

    fn render_login_question(
        &mut self,
        question: &UserInputQuestion,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut block = div().flex().flex_col().gap(px(6.0));
        block = block.child(div().child(SharedString::from(question.question.clone())));
        if question.options.is_empty() {
            block = block.child(self.login_text.clone().into_any_element());
        } else {
            let selected = self
                .login
                .as_ref()
                .and_then(|view| view.picks.get(&question.id).cloned())
                .unwrap_or_default();
            let mut chips = div().flex().flex_row().gap(px(6.0));
            for option in &question.options {
                let is_selected = selected.contains(option);
                let question_id = question.id.clone();
                let label_for_click = option.clone();
                let multi = question.multi_select;
                chips = chips.child(
                    div()
                        .id(SharedString::from(format!(
                            "omp-login-opt-{question_id}-{option}"
                        )))
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(if is_selected { theme.accent } else { theme.border })
                        .text_color(if is_selected { theme.text } else { theme.text_muted })
                        .cursor_pointer()
                        .child(SharedString::from(option.clone()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(view) = &mut this.login {
                                let picks = view.picks.entry(question_id.clone()).or_default();
                                if multi {
                                    if let Some(pos) =
                                        picks.iter().position(|p| p == &label_for_click)
                                    {
                                        picks.remove(pos);
                                    } else {
                                        picks.push(label_for_click.clone());
                                    }
                                } else {
                                    *picks = vec![label_for_click.clone()];
                                }
                            }
                            cx.notify();
                        })),
                );
            }
            block = block.child(chips);
        }
        block.into_any_element()
    }

    fn render_hub(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = match &self.catalog {
            Loadable::Ready(snapshot) => Some(snapshot.clone()),
            _ => None,
        };
        let mut outer = div().flex().flex_col().gap(px(8.0));
        if let Some(error) = self.catalog.error() {
            outer = outer.child(
                div()
                    .text_color(theme.danger)
                    .child(SharedString::from(format!("Model catalog unavailable: {error}"))),
            );
        }
        let Some(snapshot) = snapshot else {
            return widgets::section(theme, "Model hub", outer).into_any_element();
        };
        if snapshot.stale {
            outer = outer.child(
                div()
                    .text_color(theme.warning)
                    .child(SharedString::from(format!(
                        "Showing the last-known catalog — live discovery failed: {}",
                        snapshot.error.as_deref().unwrap_or("unknown error")
                    ))),
            );
        }
        if let Some(role) = &self.assigning_role {
            outer = outer.child(
                div().flex().flex_row().items_center().gap(px(8.0)).child(
                    widgets::badge_active(theme, SharedString::from(format!("Assigning role: {role} — pick a model below"))),
                ).child(
                    small_button(theme, "omp-assign-cancel", "Cancel")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.assigning_role = None;
                            cx.notify();
                        })),
                ),
            );
        }
        outer = outer.child(self.search.clone().into_any_element());
        let query = self.search.read(cx).text().to_string();
        let providers = provider_counts(&snapshot.models);
        let mut provider_row = div().flex().flex_row().gap(px(6.0));
        provider_row = provider_row.child(
            small_button(theme, "omp-provider-all", format!("All {}", snapshot.models.len()))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.provider_filter = None;
                    cx.notify();
                })),
        );
        for (provider, count) in &providers {
            let provider_for_click = provider.clone();
            provider_row = provider_row.child(
                small_button(
                    theme,
                    SharedString::from(format!("omp-provider-{provider}")),
                    format!("{provider} {count}"),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.provider_filter = Some(provider_for_click.clone());
                    cx.notify();
                })),
            );
        }
        outer = outer.child(provider_row);
        let visible = visible_models(&snapshot.models, self.provider_filter.as_deref(), &query);
        let mut list = div().flex().flex_col().gap(px(2.0));
        for model in visible.iter().take(120) {
            let selector = model.model.to_string();
            let is_selected = self.selected.as_deref() == Some(selector.as_str());
            let assigning = self.assigning_role.clone();
            let click_selector = selector.clone();
            let roles_line = roles_for_model(&self.roles, &selector);
            list = list.child(
                div()
                    .id(SharedString::from(format!("omp-model-{selector}")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .when(is_selected, |el| el.bg(theme.surface_raised))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(role) = &assigning {
                            this.assign_role(role.clone(), click_selector.clone(), cx);
                        } else {
                            this.selected = Some(click_selector.clone());
                            cx.notify();
                        }
                    }))
                    .child(div().flex_1().child(SharedString::from(selector.clone())))
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .child(context_label(model.context_window)),
                    )
                    .when(!roles_line.is_empty(), |el| {
                        el.child(widgets::badge(theme, SharedString::from(roles_line)))
                    })
                    .when(model.reasoning, |el| {
                        el.child(widgets::badge(theme, "reasoning"))
                    }),
            );
        }
        outer = outer.child(list);
        if let Some(selected) = snapshot
            .models
            .iter()
            .find(|m| Some(m.model.to_string()) == self.selected)
        {
            outer = outer.child(self.render_inspector(selected, theme, cx));
        }
        widgets::section(theme, "Model hub", outer).into_any_element()
    }

    fn render_inspector(
        &mut self,
        model: &OmpModelDescriptor,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selector = model.model.to_string();
        let mut card = widgets::section_card(theme).flex().flex_col().gap(px(6.0));
        card = card
            .child(div().child(SharedString::from(model.label.clone())))
            .child(
                div()
                    .text_color(theme.text_muted)
                    .child(SharedString::from(selector.clone())),
            )
            .child(div().child(SharedString::from(format!(
                "Context {} · Max output {} · Input {} · Reasoning {}",
                context_label(model.context_window),
                context_label(model.max_output_tokens),
                if model.input_modalities.is_empty() {
                    "text".to_string()
                } else {
                    model.input_modalities.join(" · ")
                },
                if model.reasoning { "yes" } else { "no" }
            ))));
        if !model.thinking_levels.is_empty() {
            card = card.child(div().child(SharedString::from(format!(
                "Thinking levels: {}",
                model
                    .thinking_levels
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))));
        }
        card = card.child(
            div()
                .text_color(theme.text_muted)
                .child("Roles are persistent (OMP config). Switching the model for the current session is the composer picker's job and never writes here."),
        );
        if let Some(roles) = self.roles.ready() {
            let mut chips = div().flex().flex_row().gap(px(6.0));
            for role in &roles.roles {
                let assigned_to_this = roles.assignments.get(role).is_some_and(|s| s == &selector);
                let role_for_click = role.clone();
                let selector_for_click = selector.clone();
                chips = chips.child(
                    div()
                        .id(SharedString::from(format!("omp-role-chip-{role}")))
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(if assigned_to_this {
                            theme.accent
                        } else {
                            theme.border
                        })
                        .text_color(if assigned_to_this {
                            theme.text
                        } else {
                            theme.text_muted
                        })
                        .cursor_pointer()
                        .child(SharedString::from(role.clone()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.assigning_role = Some(role_for_click.clone());
                            this.selected = Some(selector_for_click.clone());
                            cx.notify();
                        })),
                );
            }
            card = card.child(chips);
            card = card.child(
                div()
                    .text_color(theme.text_muted)
                    .child("Pick a role chip, then a model, to assign persistently."),
            );
        }
        card.into_any_element()
    }

    fn render_roles(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut outer = div().flex().flex_col().gap(px(8.0));
        match &self.roles {
            Loadable::Ready(roles) => {
                outer = outer.child(div().child(SharedString::from(format!(
                    "Assignment storage: {} · Default thinking: {}",
                    roles.storage.as_deref().unwrap_or("global"),
                    roles.default_thinking_level.as_deref().unwrap_or("OMP default")
                ))));
                for level in ["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
                    let is_current = roles.default_thinking_level.as_deref() == Some(level);
                    outer = outer.child(
                        small_button(
                            theme,
                            SharedString::from(format!("omp-thinking-{level}")),
                            if is_current {
                                format!("thinking: {level} ✓")
                            } else {
                                format!("thinking: {level}")
                            },
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.call_roles(
                                methods::OMP_SET_DEFAULT_THINKING,
                                serde_json::json!({ "level": level }),
                                cx,
                            );
                        })),
                    );
                }
                outer = outer.child(div().child(SharedString::from(format!(
                    "Cycle order: {}",
                    if roles.cycle_order.is_empty() {
                        "OMP default".to_string()
                    } else {
                        roles.cycle_order.join(" → ")
                    }
                ))));
                for role in &roles.roles {
                    let selector = roles
                        .assignments
                        .get(role)
                        .cloned()
                        .unwrap_or_else(|| "(automatic)".to_string());
                    let chain = roles
                        .fallback_chains
                        .get(role)
                        .map(|chain| format!(" · fallback: {}", chain.join(", ")))
                        .unwrap_or_default();
                    let role_for_clear = role.clone();
                    let role_for_assign_button = role.clone();
                    outer = outer.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(div().flex_1().child(SharedString::from(format!(
                                "{role} → {selector}{chain}"
                            ))))
                            .child(
                                small_button(
                                    theme,
                                    SharedString::from(format!("omp-role-assign-{role}")),
                                    "Assign…",
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.assigning_role = Some(role_for_assign_button.clone());
                                    cx.notify();
                                })),
                            )
                            .child(
                                small_button(
                                    theme,
                                    SharedString::from(format!("omp-role-clear-{role}")),
                                    "Clear",
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.clear_role(role_for_clear.clone(), cx);
                                })),
                            ),
                    );
                }
                // Cycle + fallback writes ride the same persisted
                // snapshot: "Cycle +" appends the role to the cycle
                // order; fallback buttons use the hub's selected model.
                for role in &roles.roles {
                    let mut order = roles.cycle_order.clone();
                    if !order.contains(role) {
                        order.push(role.clone());
                    }
                    let role_for_cycle = role.clone();
                    let role_for_fallback = role.clone();
                    let role_for_fallback_clear = role.clone();
                    let selected = self.selected.clone();
                    outer = outer.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(div().flex_1().child(SharedString::from(format!("{role} cycle/fallback"))))
                            .child(
                                small_button(
                                    theme,
                                    SharedString::from(format!("omp-cycle-add-{role}")),
                                    "Cycle +",
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let _ = &role_for_cycle;
                                    this.call_roles(
                                        methods::OMP_SET_CYCLE_ORDER,
                                        serde_json::json!({ "roles": order }),
                                        cx,
                                    );
                                })),
                            )
                            .child(
                                small_button(
                                    theme,
                                    SharedString::from(format!("omp-fallback-set-{role}")),
                                    "Fallback = selected model",
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(selector) = &selected {
                                        this.call_roles(
                                            methods::OMP_SET_FALLBACK_CHAIN,
                                            serde_json::json!({
                                                "key": role_for_fallback,
                                                "chain": [selector],
                                            }),
                                            cx,
                                        );
                                    }
                                })),
                            )
                            .child(
                                small_button(
                                    theme,
                                    SharedString::from(format!("omp-fallback-clear-{role}")),
                                    "Clear fallback",
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.call_roles(
                                        methods::OMP_SET_FALLBACK_CHAIN,
                                        serde_json::json!({
                                            "key": role_for_fallback_clear,
                                            "chain": [],
                                        }),
                                        cx,
                                    );
                                })),
                            ),
                    );
                }
            }
            Loadable::Error(error) => {
                outer = outer.child(
                    div()
                        .text_color(theme.danger)
                        .child(SharedString::from(error.clone())),
                );
            }
            _ => {
                outer = outer.child(div().text_color(theme.text_muted).child("Loading roles…"));
            }
        }
        if let Some(limits) = self.limits.ready() {
            outer = outer.child(div().child(SharedString::from(format!(
                "Provider limits: {} report(s), {} account(s) without usage, {} disabled credential(s)",
                limits.reports.len(),
                limits.accounts_without_usage.len(),
                limits.disabled_credentials.len()
            ))));
            for report in &limits.reports {
                let label = report
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .or_else(|| report.get("providerId").and_then(|v| v.as_str()))
                    .unwrap_or("provider")
                    .to_string();
                outer = outer.child(
                    div()
                        .text_color(theme.text_muted)
                        .child(SharedString::from(format!("{label}: usage reported by OMP"))),
                );
            }
        }
        // Fallback-chain editor for the selected role: a text field
        // is overkill here; the persistent write is one click per
        // selected model from the inspector's role chips, and chains
        // are set for the role named by the current assignment.
        widgets::section(theme, "Roles, thinking & limits", outer).into_any_element()
    }
}

impl Render for ModelsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::toast::take_error(&mut self.error, cx);
        let theme = Theme::of(cx).for_settings_surface();
        let onboarding = self.render_onboarding(&theme, cx);
        let hub = self.render_hub(&theme, cx);
        let roles = self.render_roles(&theme, cx);
        div()
            .id("models-page")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll.scroll)
            .child(
                widgets::page_column()
                    .child(widgets::page_header(&theme, "Models", None))
                    .child(widgets::page_subtitle(
                        &theme,
                        "OMP models, persistent roles and provider accounts. Session-only model switching lives in the composer picker.",
                    ))
                    .child(
                        small_button(&theme, "omp-refresh", "Refresh")
                            .on_click(cx.listener(|this, _, _, cx| this.load_all(cx))),
                    )
                    .child(onboarding)
                    .child(hub)
                    .child(roles),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(provider: &str, id: &str, label: &str) -> OmpModelDescriptor {
        OmpModelDescriptor {
            model: zeron_proto::omp::OmpModelRef::new(provider, id),
            label: label.into(),
            description: None,
            reasoning: false,
            thinking_levels: Vec::new(),
            input_modalities: vec!["text".into()],
            context_window: Some(272_000),
            max_output_tokens: None,
            native: serde_json::Value::Null,
        }
    }

    #[test]
    fn visible_models_filters_by_provider_and_search() {
        let models = vec![
            descriptor("openai", "gpt-5.2", "GPT-5.2"),
            descriptor("anthropic", "claude-opus-5", "Claude Opus"),
        ];
        assert_eq!(visible_models(&models, None, "").len(), 2);
        assert_eq!(visible_models(&models, Some("anthropic"), "").len(), 1);
        assert_eq!(visible_models(&models, None, "gpt").len(), 1);
        assert_eq!(visible_models(&models, None, "OPUS").len(), 1);
        assert!(visible_models(&models, Some("openai"), "opus").is_empty());
    }

    #[test]
    fn provider_counts_and_context_labels() {
        let models = vec![
            descriptor("openai", "a", "A"),
            descriptor("openai", "b", "B"),
            descriptor("apple", "c", "C"),
        ];
        assert_eq!(
            provider_counts(&models),
            vec![("openai".to_string(), 2), ("apple".to_string(), 1)]
        );
        assert_eq!(context_label(Some(1_000_000)), "1M");
        assert_eq!(context_label(Some(272_000)), "272k");
        assert_eq!(context_label(None), "—");
    }
}
