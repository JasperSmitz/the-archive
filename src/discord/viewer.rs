//! Signed component navigation data, never an authorization principal or a private session.
use super::config;
use crate::app::retrieval::Direction;
use serde_json::{Value, json};
#[derive(Clone, Copy, Debug)]
pub struct Navigation {
    pub character: i64,
    pub image: i64,
    pub direction: Direction,
    pub message: u64,
}
pub fn custom_id(character: i64, image: i64, direction: Direction) -> String {
    format!(
        "av1:{character}:{image}:{}",
        if direction == Direction::Previous {
            "p"
        } else {
            "n"
        }
    )
}
fn cursor(value: &str) -> Option<(i64, i64, Direction)> {
    if value.len() > 100 {
        return None;
    }
    let parts: Vec<_> = value.split(':').collect();
    if parts.len() != 4 || parts[0] != "av1" {
        return None;
    }
    let positive = |s: &str| {
        s.parse::<i64>()
            .ok()
            .filter(|id| *id > 0 && id.to_string() == s)
    };
    let character = positive(parts[1])?;
    let image = positive(parts[2])?;
    let direction = match parts[3] {
        "p" => Direction::Previous,
        "n" => Direction::Next,
        _ => return None,
    };
    Some((character, image, direction))
}
pub fn controls(character: i64, image: i64, previous: bool, next: bool) -> Value {
    json!([{"type":1,"components":[
        {"type":2,"style":2,"label":"Previous","custom_id":custom_id(character,image,Direction::Previous),"disabled":!previous},
        {"type":2,"style":2,"label":"Next","custom_id":custom_id(character,image,Direction::Next),"disabled":!next}
    ]}])
}
pub fn parse(payload: &Value, application: u64) -> Option<Navigation> {
    if payload.pointer("/data/component_type")?.as_u64()? != 2 {
        return None;
    }
    let submitted = payload.pointer("/data/custom_id")?.as_str()?;
    let (character, image, direction) = cursor(submitted)?;
    let message = payload.get("message")?;
    let id = config::snowflake(message.get("id")?.as_str()?).ok()?;
    // App-owned interaction webhook messages, not arbitrary bot/user messages.
    if config::snowflake(message.get("application_id")?.as_str()?).ok()? != application
        || config::snowflake(message.pointer("/author/id")?.as_str()?).ok()? != application
        || !message.pointer("/author/bot")?.as_bool()?
    {
        return None;
    }
    if let Some(webhook) = message.get("webhook_id")
        && config::snowflake(webhook.as_str()?).ok()? != application
    {
        return None;
    }
    let flags = match message.get("flags") {
        None => 0,
        Some(flags) => flags.as_u64()?,
    };
    if flags & (64 | 32768) != 0 {
        return None;
    }
    let channel = config::snowflake(
        payload
            .get("channel_id")
            .or_else(|| payload.pointer("/channel/id"))?
            .as_str()?,
    )
    .ok()?;
    if let Some(nested_channel) = payload.pointer("/channel/id")
        && config::snowflake(nested_channel.as_str()?).ok()? != channel
    {
        return None;
    }
    if config::snowflake(message.get("channel_id")?.as_str()?).ok()? != channel {
        return None;
    }
    let rows = message.get("components")?.as_array()?;
    if rows.len() != 1 || rows[0].get("type")?.as_u64()? != 1 {
        return None;
    }
    let buttons = rows[0].get("components")?.as_array()?;
    if buttons.len() != 2 {
        return None;
    }
    for (button, expected, label) in [
        (&buttons[0], Direction::Previous, "Previous"),
        (&buttons[1], Direction::Next, "Next"),
    ] {
        if button.get("type")?.as_u64()? != 2
            || button.get("style")?.as_u64()? != 2
            || button.get("label")?.as_str()? != label
            || button.get("custom_id")?.as_str()? != custom_id(character, image, expected)
            || button.get("url").is_some()
        {
            return None;
        }
        let disabled = match button.get("disabled") {
            None => false,
            Some(value) => value.as_bool()?,
        };
        if expected == direction && disabled {
            return None;
        }
    }
    Some(Navigation {
        character,
        image,
        direction,
        message: id,
    })
}
