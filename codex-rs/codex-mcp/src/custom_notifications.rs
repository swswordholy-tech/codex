use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use codex_protocol::protocol::Op;
use codex_protocol::protocol::Submission;
use codex_protocol::user_input::UserInput;
use rmcp::model::CustomNotification;
use serde::Deserialize;

const CHAT_CHANNEL_NOTIFICATION_METHOD: &str = "notifications/chat/channel";

#[derive(Debug, Deserialize)]
pub(crate) struct ChannelNotificationMeta {
    chat_id: Option<String>,
    sender_id: Option<String>,
    message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChannelNotificationParams {
    content: String,
    meta: Option<ChannelNotificationMeta>,
}

pub(crate) fn submission_for_custom_notification(
    server_name: &str,
    notification: CustomNotification,
) -> Option<Submission> {
    if notification.method != CHAT_CHANNEL_NOTIFICATION_METHOD {
        return None;
    }

    let params = notification.params?;
    let parsed: ChannelNotificationParams = serde_json::from_value(params).ok()?;
    let text = format_channel_message(server_name, &parsed)?;
    Some(Submission {
        id: next_submission_id(
            server_name,
            parsed.meta.as_ref().and_then(|meta| meta.message_id.as_deref()),
        ),
        op: Op::UserInput {
            items: vec![UserInput::Text {
                text,
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        },
        trace: None,
    })
}

fn format_channel_message(server_name: &str, params: &ChannelNotificationParams) -> Option<String> {
    let content = params.content.trim();
    if content.is_empty() {
        return None;
    }

    if content.starts_with("<channel ") {
        return Some(content.to_string());
    }

    let chat_id = params
        .meta
        .as_ref()
        .and_then(|meta| meta.chat_id.as_deref())
        .map(xml_escape_attr)
        .unwrap_or_default();
    let sender_id = params
        .meta
        .as_ref()
        .and_then(|meta| meta.sender_id.as_deref())
        .map(xml_escape_attr)
        .unwrap_or_default();
    let message_id = params
        .meta
        .as_ref()
        .and_then(|meta| meta.message_id.as_deref())
        .map(xml_escape_attr);

    let mut tag = format!(
        "<channel source=\"{}\" chat_id=\"{}\" sender_id=\"{}\"",
        xml_escape_attr(&channel_source_for_server(server_name)),
        chat_id,
        sender_id,
    );
    if let Some(message_id) = message_id {
        tag.push_str(&format!(" message_id=\"{message_id}\""));
    }
    tag.push('>');

    Some(format!("{tag}\n{content}\n</channel>"))
}

fn xml_escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn channel_source_for_server(server_name: &str) -> String {
    format!("mcp:{server_name}")
}

fn next_submission_id(server_name: &str, message_id: Option<&str>) -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    match message_id {
        Some(message_id) if !message_id.is_empty() => {
            format!("mcp-custom-notification-{server_name}-{message_id}-{nonce}")
        }
        _ => format!("mcp-custom-notification-{server_name}-{nonce}"),
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rmcp::model::CustomNotification;
    use rmcp::model::Extensions;
    use serde_json::json;

    use super::*;

    #[test]
    fn wraps_chat_channel_notification_as_channel_input() {
        let submission = submission_for_custom_notification(
            "agentchat",
            CustomNotification {
                method: "notifications/chat/channel".to_string(),
                params: Some(json!({
                    "content": "hello",
                    "meta": {
                        "chat_id": "chat-1",
                        "sender_id": "sender-1",
                        "message_id": "msg-1",
                    }
                })),
                extensions: Extensions::new(),
            },
        )
        .expect("submission");

        let Op::UserInput { items, .. } = submission.op else {
            panic!("expected user input op");
        };
        assert_eq!(
            items,
            vec![UserInput::Text {
                text: "<channel source=\"mcp:agentchat\" chat_id=\"chat-1\" sender_id=\"sender-1\" message_id=\"msg-1\">\nhello\n</channel>".to_string(),
                text_elements: Vec::new(),
            }]
        );
    }

    #[test]
    fn preserves_existing_channel_wrapper() {
        let wrapped =
            "<channel source=\"plugin:agentchat:agentchat\" chat_id=\"chat-1\" sender_id=\"sender-1\">\nhello\n</channel>";
        let submission = submission_for_custom_notification(
            "agentchat",
            CustomNotification {
                method: "notifications/chat/channel".to_string(),
                params: Some(json!({
                    "content": wrapped
                })),
                extensions: Extensions::new(),
            },
        )
        .expect("submission");

        let Op::UserInput { items, .. } = submission.op else {
            panic!("expected user input op");
        };
        assert_eq!(
            items,
            vec![UserInput::Text {
                text: wrapped.to_string(),
                text_elements: Vec::new(),
            }]
        );
    }

    #[test]
    fn carries_all_channel_metadata_into_session_text() {
        let submission = submission_for_custom_notification(
            "chatbridge",
            CustomNotification {
                method: "notifications/chat/channel".to_string(),
                params: Some(json!({
                    "content": "hello",
                    "meta": {
                        "chat_id": "room-42",
                        "sender_id": "alice",
                        "message_id": "msg-99",
                    }
                })),
                extensions: Extensions::new(),
            },
        )
        .expect("submission");

        let Op::UserInput { items, .. } = submission.op else {
            panic!("expected user input op");
        };
        let [UserInput::Text { text, .. }] = items.as_slice() else {
            panic!("expected one text item");
        };

        assert!(text.contains("source=\"mcp:chatbridge\""));
        assert!(text.contains("chat_id=\"room-42\""));
        assert!(text.contains("sender_id=\"alice\""));
        assert!(text.contains("message_id=\"msg-99\""));
    }

    #[test]
    fn emits_empty_optional_fields_when_meta_is_missing() {
        let submission = submission_for_custom_notification(
            "chatbridge",
            CustomNotification {
                method: "notifications/chat/channel".to_string(),
                params: Some(json!({
                    "content": "hello"
                })),
                extensions: Extensions::new(),
            },
        )
        .expect("submission");

        let Op::UserInput { items, .. } = submission.op else {
            panic!("expected user input op");
        };
        assert_eq!(
            items,
            vec![UserInput::Text {
                text: "<channel source=\"mcp:chatbridge\" chat_id=\"\" sender_id=\"\">\nhello\n</channel>"
                    .to_string(),
                text_elements: Vec::new(),
            }]
        );
    }

    #[test]
    fn ignores_non_chat_channel_notifications() {
        assert!(
            submission_for_custom_notification(
                "agentchat",
                CustomNotification {
                    method: "notifications/codex/channel".to_string(),
                    params: Some(serde_json::Value::Null),
                    extensions: Extensions::new(),
                },
            )
            .is_none()
        );
    }
}
