//! Domain model types for the contact and outreach system.
//!
//! Contains `Contact`, `OutreachStrategy`, `OutreachLog`, `ProfileEntry`, and the
//! supporting enums `Importance`, `ContactMethod`, `ProfileSource`, `ProfileCategory`,
//! and `StrategyType`.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Importance {
    Low,
    Medium,
    High,
    VeryHigh,
}

impl Importance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::VeryHigh => "veryhigh",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "low" => Self::Low,
            "high" => Self::High,
            "veryhigh" | "very_high" | "very-high" => Self::VeryHigh,
            _ => Self::Medium,
        }
    }
}

impl fmt::Display for Importance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContactMethod {
    Email,
    Telegram,
    WhatsApp,
    Signal,
    Facebook,
    LinkedIn,
}

impl ContactMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Telegram => "telegram",
            Self::WhatsApp => "whatsapp",
            Self::Signal => "signal",
            Self::Facebook => "facebook",
            Self::LinkedIn => "linkedin",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "telegram" => Self::Telegram,
            "whatsapp" => Self::WhatsApp,
            "signal" => Self::Signal,
            "facebook" => Self::Facebook,
            "linkedin" => Self::LinkedIn,
            _ => Self::Email,
        }
    }
}

impl fmt::Display for ContactMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub first_name: String,
    pub last_name: String,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub telegram: Option<String>,
    pub whatsapp: Option<String>,
    pub signal: Option<String>,
    pub facebook: Option<String>,
    pub linkedin: Option<String>,
    pub preferred_contact_method: ContactMethod,
    pub birthday: Option<String>,
    pub importance: Importance,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Contact {
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSource {
    LinkedIn,
    Facebook,
    News,
    Manual,
}

impl ProfileSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LinkedIn => "linkedin",
            Self::Facebook => "facebook",
            Self::News => "news",
            Self::Manual => "manual",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "linkedin" => Self::LinkedIn,
            "facebook" => Self::Facebook,
            "manual" => Self::Manual,
            _ => Self::News,
        }
    }
}

impl fmt::Display for ProfileSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileCategory {
    WorkHistory,
    Education,
    Achievement,
    News,
    Personal,
}

impl ProfileCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WorkHistory => "work_history",
            Self::Education => "education",
            Self::Achievement => "achievement",
            Self::News => "news",
            Self::Personal => "personal",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().replace('-', "_").as_str() {
            "work_history" => Self::WorkHistory,
            "education" => Self::Education,
            "achievement" => Self::Achievement,
            "personal" => Self::Personal,
            _ => Self::News,
        }
    }
}

impl fmt::Display for ProfileCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileEntry {
    pub id: String,
    pub contact_id: String,
    pub source: ProfileSource,
    pub category: ProfileCategory,
    pub content: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyType {
    BirthdayMessage,
    NewsSearch,
    DraftCongratulations,
    PeriodicCheckin,
    ProfileScrape,
}

impl StrategyType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BirthdayMessage => "birthday_message",
            Self::NewsSearch => "news_search",
            Self::DraftCongratulations => "draft_congratulations",
            Self::PeriodicCheckin => "periodic_checkin",
            Self::ProfileScrape => "profile_scrape",
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "birthday_message" => Some(Self::BirthdayMessage),
            "news_search" => Some(Self::NewsSearch),
            "draft_congratulations" => Some(Self::DraftCongratulations),
            "periodic_checkin" => Some(Self::PeriodicCheckin),
            "profile_scrape" => Some(Self::ProfileScrape),
            _ => None,
        }
    }
}

impl fmt::Display for StrategyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutreachStrategy {
    pub id: String,
    pub contact_id: String,
    pub strategy_type: StrategyType,
    pub enabled: bool,
    pub interval_days: Option<i64>,
    pub last_executed: Option<String>,
    pub config_json: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutreachLog {
    pub id: String,
    pub contact_id: String,
    pub strategy_type: StrategyType,
    pub executed_at: String,
    pub result: String,
    pub details: Option<String>,
}
