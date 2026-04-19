//! Dry-run layer for `zad gcal`.
//!
//! The CLI holds a `Box<dyn GcalTransport>` so a `--dry-run`
//! invocation never touches the network (or the keychain) — the live
//! impl delegates to [`GcalHttp`] and the preview impl emits
//! [`DryRunOp`] records to a shared sink.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::error::Result;
use crate::service::gcal::client::{Calendar, CalendarEntry, Event, EventsListParams, GcalHttp};
use crate::service::{DryRunOp, DryRunSink};

/// Runtime surface of the Google Calendar service. One method per verb
/// reachable from `zad gcal …`.
#[async_trait]
pub trait GcalTransport: Send + Sync {
    async fn list_calendars(&self) -> Result<Vec<CalendarEntry>>;
    async fn get_calendar(&self, calendar_id: &str) -> Result<Calendar>;
    async fn list_events(&self, calendar_id: &str, params: &EventsListParams)
    -> Result<Vec<Event>>;
    async fn get_event(&self, calendar_id: &str, event_id: &str) -> Result<Event>;
    async fn create_event(
        &self,
        calendar_id: &str,
        body: &serde_json::Value,
        send_updates: Option<&str>,
    ) -> Result<Event>;
    async fn update_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        body: &serde_json::Value,
        send_updates: Option<&str>,
    ) -> Result<Event>;
    async fn delete_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        send_updates: Option<&str>,
    ) -> Result<()>;
}

#[async_trait]
impl GcalTransport for GcalHttp {
    async fn list_calendars(&self) -> Result<Vec<CalendarEntry>> {
        GcalHttp::list_calendars(self).await
    }
    async fn get_calendar(&self, calendar_id: &str) -> Result<Calendar> {
        GcalHttp::get_calendar(self, calendar_id).await
    }
    async fn list_events(
        &self,
        calendar_id: &str,
        params: &EventsListParams,
    ) -> Result<Vec<Event>> {
        GcalHttp::list_events(self, calendar_id, params).await
    }
    async fn get_event(&self, calendar_id: &str, event_id: &str) -> Result<Event> {
        GcalHttp::get_event(self, calendar_id, event_id).await
    }
    async fn create_event(
        &self,
        calendar_id: &str,
        body: &serde_json::Value,
        send_updates: Option<&str>,
    ) -> Result<Event> {
        GcalHttp::create_event(self, calendar_id, body, send_updates).await
    }
    async fn update_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        body: &serde_json::Value,
        send_updates: Option<&str>,
    ) -> Result<Event> {
        GcalHttp::update_event(self, calendar_id, event_id, body, send_updates).await
    }
    async fn delete_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        send_updates: Option<&str>,
    ) -> Result<()> {
        GcalHttp::delete_event(self, calendar_id, event_id, send_updates).await
    }
}

/// Preview transport used when the caller passed `--dry-run`.
///
/// Intercepts every *mutating* verb (create / update / delete) by
/// emitting a [`DryRunOp`] and returning a stub success value — a
/// fake [`Event`] with `id = "dry-run"` for create/update, `Ok(())`
/// for delete. Read verbs return empty results so `--dry-run` works
/// without credentials.
pub struct DryRunGcalTransport {
    sink: Arc<dyn DryRunSink>,
}

impl DryRunGcalTransport {
    pub fn new(sink: Arc<dyn DryRunSink>) -> Self {
        Self { sink }
    }

    fn record(&self, verb: &'static str, summary: String, details: serde_json::Value) {
        self.sink.record(DryRunOp {
            service: "gcal",
            verb,
            summary,
            details,
        });
    }
}

#[async_trait]
impl GcalTransport for DryRunGcalTransport {
    async fn list_calendars(&self) -> Result<Vec<CalendarEntry>> {
        Ok(vec![])
    }

    async fn get_calendar(&self, calendar_id: &str) -> Result<Calendar> {
        Ok(Calendar {
            id: calendar_id.to_string(),
            summary: "(dry-run)".into(),
            time_zone: Some("UTC".into()),
        })
    }

    async fn list_events(
        &self,
        _calendar_id: &str,
        _params: &EventsListParams,
    ) -> Result<Vec<Event>> {
        Ok(vec![])
    }

    async fn get_event(&self, _calendar_id: &str, event_id: &str) -> Result<Event> {
        Ok(Event {
            id: event_id.to_string(),
            summary: Some("(dry-run)".into()),
            description: None,
            location: None,
            start: None,
            end: None,
            attendees: None,
            html_link: None,
            status: Some("confirmed".into()),
        })
    }

    async fn create_event(
        &self,
        calendar_id: &str,
        body: &serde_json::Value,
        send_updates: Option<&str>,
    ) -> Result<Event> {
        let summary = body
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("(no summary)");
        self.record(
            "create_event",
            format!("would create event `{summary}` on calendar `{calendar_id}`"),
            json!({
                "command": "gcal.events.create",
                "calendar": calendar_id,
                "send_updates": send_updates,
                "event": body,
            }),
        );
        Ok(Event {
            id: "dry-run".into(),
            summary: Some(summary.to_string()),
            description: body
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            location: body
                .get("location")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            start: None,
            end: None,
            attendees: None,
            html_link: None,
            status: Some("confirmed".into()),
        })
    }

    async fn update_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        body: &serde_json::Value,
        send_updates: Option<&str>,
    ) -> Result<Event> {
        self.record(
            "update_event",
            format!("would patch event `{event_id}` on calendar `{calendar_id}`"),
            json!({
                "command": "gcal.events.update",
                "calendar": calendar_id,
                "event_id": event_id,
                "send_updates": send_updates,
                "patch": body,
            }),
        );
        Ok(Event {
            id: event_id.to_string(),
            summary: body
                .get("summary")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            description: body
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            location: body
                .get("location")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            start: None,
            end: None,
            attendees: None,
            html_link: None,
            status: Some("confirmed".into()),
        })
    }

    async fn delete_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        send_updates: Option<&str>,
    ) -> Result<()> {
        self.record(
            "delete_event",
            format!("would delete event `{event_id}` on calendar `{calendar_id}`"),
            json!({
                "command": "gcal.events.delete",
                "calendar": calendar_id,
                "event_id": event_id,
                "send_updates": send_updates,
            }),
        );
        Ok(())
    }
}
