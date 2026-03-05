//! Standalone rolodex app binary.
//!
//! Communicates with the host via the CCPP protocol over stdin/stdout using
//! [`corre_sdk::AppClient`]. This binary has no dependency on `corre-core`
//! — it uses only `corre-sdk` types and utilities.

use anyhow::Context as _;
use corre_sdk::tools::{extract_json, normalize_freshness, parse_search_results};
use corre_sdk::types::{AppOutput, Article, Section, Source};
use corre_sdk::{AppClient, LlmMessage, LlmRequest, LlmRole};
use rolodex::db::profiles::new_profile_entry;
use rolodex::db::{Contact, Database, Importance, ProfileCategory, ProfileEntry, ProfileSource, StrategyType};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;

// ---------------------------------------------------------------------------
// YAML config model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
struct RolodexConfigFile {
    rolodex: RolodexConfig,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct RolodexConfig {
    #[serde(default = "default_freshness")]
    news_search_freshness: String,
    #[serde(default = "default_max_news")]
    max_news_per_contact: usize,
    #[serde(default = "default_birthday_style")]
    birthday_message_style: String,
    #[serde(default = "default_checkin_style")]
    checkin_message_style: String,
}

impl Default for RolodexConfig {
    fn default() -> Self {
        Self {
            news_search_freshness: default_freshness(),
            max_news_per_contact: default_max_news(),
            birthday_message_style: default_birthday_style(),
            checkin_message_style: default_checkin_style(),
        }
    }
}

fn default_freshness() -> String {
    "1w".into()
}
fn default_max_news() -> usize {
    5
}
fn default_birthday_style() -> String {
    "warm".into()
}
fn default_checkin_style() -> String {
    "casual".into()
}

fn load_config(config_dir: &std::path::Path, config_path: Option<&str>) -> RolodexConfig {
    let Some(config_path) = config_path else {
        return RolodexConfig::default();
    };
    let path = config_dir.join(config_path);
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_yaml_ng::from_str::<RolodexConfigFile>(&content) {
            Ok(file) => file.rolodex,
            Err(e) => {
                tracing::warn!("Failed to parse rolodex config at {}: {e}, using defaults", path.display());
                RolodexConfig::default()
            }
        },
        Err(e) => {
            tracing::info!("No rolodex config at {}: {e}, using defaults", path.display());
            RolodexConfig::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("ERROR rolodex failed: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let client = Arc::new(AppClient::from_stdio());
    let params = client.accept_initialize().await?;
    let _guard = corre_sdk::init_tracing(&params.app_name, params.log_dir.as_deref(), params.log_level.as_deref());

    let config_dir = PathBuf::from(&params.config_dir);
    let max_concurrent_llm = params.max_concurrent_llm;

    let config = load_config(&config_dir, params.config_path.as_deref());
    let db_path = config_dir.join("rolodex.db");
    let db = Database::open(&db_path).with_context(|| format!("failed to open rolodex database at {}", db_path.display()))?;

    let now = chrono::Utc::now();
    let today = now.date_naive();
    let semaphore = Arc::new(Semaphore::new(max_concurrent_llm));
    let mut all_sections: Vec<Section> = Vec::new();

    // --- Step 1: Birthday articles ---
    client.report_progress("checking_birthdays", Some(10), None).await?;
    let birthday_contacts = db.birthdays_on(&today)?;
    if !birthday_contacts.is_empty() {
        tracing::info!("Found {} contacts with birthdays today", birthday_contacts.len());
        let mut birthday_articles = Vec::new();

        let handles: Vec<_> = birthday_contacts
            .iter()
            .map(|contact| {
                let sem = semaphore.clone();
                let client = client.clone();
                let style = config.birthday_message_style.clone();
                let name = contact.full_name();
                let notes = contact.notes.clone().unwrap_or_default();
                let importance = contact.importance;
                let birthday = contact.birthday.clone().unwrap_or_default();
                let profile_context =
                    db.get_profile_entries(&contact.id, 8).map(|entries| format_profile_context(&entries)).unwrap_or_default();

                async move {
                    let _permit = sem.acquire().await.unwrap();
                    generate_birthday_card(&client, &name, &birthday, &notes, importance, &style, &profile_context).await
                }
            })
            .collect();

        let results = futures::future::join_all(handles).await;
        for (article, contact) in results.into_iter().zip(birthday_contacts.iter()) {
            match article {
                Ok(article) => {
                    birthday_articles.push(article);
                    if let Ok(strategies) = db.get_strategies_for_contact(&contact.id) {
                        for s in strategies.iter().filter(|s| s.strategy_type == StrategyType::BirthdayMessage) {
                            let _ = db.mark_strategy_executed(&s.id, &now);
                        }
                    }
                }
                Err(e) => tracing::warn!("Failed to generate birthday article for {}: {e}", contact.full_name()),
            }
        }

        if !birthday_articles.is_empty() {
            all_sections.push(Section { title: "Birthdays Today".into(), articles: birthday_articles });
        }
    }

    // --- Step 2: News search for contacts with due NewsSearch strategy ---
    client.report_progress("searching_contact_news", Some(30), None).await?;
    let news_strategies = db.strategies_due_by_type(StrategyType::NewsSearch, &now)?;
    if !news_strategies.is_empty() {
        tracing::info!("Found {} due news search strategies", news_strategies.len());
        let mut news_articles = Vec::new();

        let handles: Vec<_> = news_strategies
            .iter()
            .filter_map(|strategy| {
                let contact = db.get_contact(&strategy.contact_id).ok().flatten()?;
                Some((strategy.clone(), contact))
            })
            .map(|(strategy, contact)| {
                let sem = semaphore.clone();
                let client = client.clone();
                let freshness = normalize_freshness(&config.news_search_freshness).to_string();
                let max_results = config.max_news_per_contact;
                async move {
                    let _permit = sem.acquire().await.unwrap();
                    let result = search_contact_news(&client, &contact, &freshness, max_results).await;
                    (strategy, contact, result)
                }
            })
            .collect();

        let results = futures::future::join_all(handles).await;
        for (strategy, contact, result) in results {
            match result {
                Ok(articles) if !articles.is_empty() => {
                    news_articles.extend(articles);
                    let _ = db.mark_strategy_executed(&strategy.id, &now);

                    if let Ok(contact_strategies) = db.get_strategies_for_contact(&contact.id) {
                        for cs in contact_strategies.iter().filter(|s| s.strategy_type == StrategyType::DraftCongratulations && s.enabled) {
                            let _ = db.mark_strategy_executed(&cs.id, &now);
                        }
                    }
                }
                Ok(_) => {
                    let _ = db.mark_strategy_executed(&strategy.id, &now);
                }
                Err(e) => tracing::warn!("News search failed for {}: {e}", contact.full_name()),
            }
        }

        if !news_articles.is_empty() {
            all_sections.push(Section { title: "Contact News".into(), articles: news_articles });
        }
    }

    // --- Step 3: Periodic check-in reminders ---
    client.report_progress("generating_checkin_reminders", Some(55), None).await?;
    let checkin_strategies = db.strategies_due_by_type(StrategyType::PeriodicCheckin, &now)?;
    if !checkin_strategies.is_empty() {
        tracing::info!("Found {} due check-in strategies", checkin_strategies.len());
        let mut checkin_articles = Vec::new();

        let handles: Vec<_> = checkin_strategies
            .iter()
            .filter_map(|strategy| {
                let contact = db.get_contact(&strategy.contact_id).ok().flatten()?;
                Some((strategy.clone(), contact))
            })
            .map(|(strategy, contact)| {
                let sem = semaphore.clone();
                let client = client.clone();
                let style = config.checkin_message_style.clone();
                let profile_context =
                    db.get_profile_entries(&contact.id, 8).map(|entries| format_profile_context(&entries)).unwrap_or_default();
                async move {
                    let _permit = sem.acquire().await.unwrap();
                    let result = generate_checkin_reminder(&client, &contact, &style, &profile_context).await;
                    (strategy, contact, result)
                }
            })
            .collect();

        let results = futures::future::join_all(handles).await;
        for (strategy, contact, result) in results {
            match result {
                Ok(article) => {
                    checkin_articles.push(article);
                    let _ = db.mark_strategy_executed(&strategy.id, &now);
                }
                Err(e) => tracing::warn!("Failed to generate check-in for {}: {e}", contact.full_name()),
            }
        }

        if !checkin_articles.is_empty() {
            all_sections.push(Section { title: "Check-in Reminders".into(), articles: checkin_articles });
        }
    }

    // --- Step 4: Profile scrape for contacts with due ProfileScrape strategy ---
    client.report_progress("scraping_contact_profiles", Some(75), None).await?;
    let profile_strategies = db.strategies_due_by_type(StrategyType::ProfileScrape, &now)?;
    if !profile_strategies.is_empty() {
        tracing::info!("Found {} due profile scrape strategies", profile_strategies.len());

        let handles: Vec<_> = profile_strategies
            .iter()
            .filter_map(|strategy| {
                let contact = db.get_contact(&strategy.contact_id).ok().flatten()?;
                Some((strategy.clone(), contact))
            })
            .map(|(strategy, contact)| {
                let sem = semaphore.clone();
                let client = client.clone();
                let freshness = normalize_freshness(&config.news_search_freshness).to_string();
                let max_results = config.max_news_per_contact;
                async move {
                    let _permit = sem.acquire().await.unwrap();
                    let result = scrape_contact_profiles(&client, &contact, &freshness, max_results).await;
                    (strategy, contact, result)
                }
            })
            .collect();

        let results = futures::future::join_all(handles).await;
        for (strategy, contact, result) in results {
            match result {
                Ok(entries) => {
                    for entry in &entries {
                        if let Err(e) = db.insert_profile_entry(entry) {
                            tracing::warn!("Failed to insert profile entry for {}: {e}", contact.full_name());
                        }
                    }
                    tracing::info!("Scraped {} profile entries for {}", entries.len(), contact.full_name());
                    let _ = db.mark_strategy_executed(&strategy.id, &now);
                }
                Err(e) => tracing::warn!("Profile scrape failed for {}: {e}", contact.full_name()),
            }
        }
    }

    let total: usize = all_sections.iter().map(|s| s.articles.len()).sum();
    tracing::info!("Rolodex produced {total} articles across {} sections", all_sections.len());

    client.report_progress("sending_result", Some(95), None).await?;

    let output = AppOutput {
        app_name: "rolodex".into(),
        produced_at: now,
        sections: all_sections,
        content_type: Default::default(),
        custom_content: None,
    };

    client.send_result(output).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// LLM helper functions
// ---------------------------------------------------------------------------

/// Generate a personalized birthday article via LLM.
async fn generate_birthday_card(
    client: &AppClient<tokio::io::Stdout>,
    name: &str,
    birthday: &str,
    notes: &str,
    importance: Importance,
    style: &str,
    profile_context: &str,
) -> anyhow::Result<Article> {
    let notes_section = if notes.is_empty() { String::new() } else { format!("\nNotes about this person: {notes}") };
    let request = LlmRequest::simple(
        format!(
            "You are a personal assistant writing a {style} birthday message. \
             Write a short, personalized birthday card (2-3 paragraphs, under 200 words). \
             The tone should be {style}. Importance level: {importance}.{notes_section}{profile_context}"
        ),
        format!("Write a birthday message for {name} (birthday: {birthday})."),
    );

    let response = client.llm_complete(request).await?;
    Ok(Article {
        title: format!("Happy Birthday, {name}!"),
        summary: format!("Today is {name}'s birthday!"),
        body: response.content.trim().to_string(),
        sources: vec![],
        score: match importance {
            Importance::VeryHigh => 1.0,
            Importance::High => 0.9,
            Importance::Medium => 0.7,
            Importance::Low => 0.5,
        },
    })
}

/// Search for news about a contact and summarize results.
async fn search_contact_news(
    client: &AppClient<tokio::io::Stdout>,
    contact: &Contact,
    freshness: &str,
    max_results: usize,
) -> anyhow::Result<Vec<Article>> {
    let query = format!("\"{}\"", contact.full_name());
    tracing::info!("Searching news for: {query}");

    let args = serde_json::json!({ "query": query, "freshness": freshness });
    let results = client.call_tool("brave-search", "brave_web_search", args).await?;

    let items = parse_search_results(results);
    if items.is_empty() {
        tracing::info!("No news found for {}", contact.full_name());
        return Ok(vec![]);
    }

    let truncated: Vec<_> = items.into_iter().take(max_results).collect();
    let results_json = serde_json::to_string(&truncated)?;
    let name = contact.full_name();

    let request = LlmRequest {
        messages: vec![
            LlmMessage {
                role: LlmRole::System,
                content: format!(
                    "You are a personal assistant summarizing news about {name}. \
                     For each relevant result, write a brief summary. Skip irrelevant results. \
                     Respond with ONLY a raw JSON array: [{{\"title\": \"...\", \"summary\": \"...\", \"body\": \"...\", \"url\": \"...\"}}]"
                ),
            },
            LlmMessage { role: LlmRole::User, content: format!("Summarize these search results about {name}:\n{results_json}") },
        ],
        temperature: Some(0.2),
        max_completion_tokens: Some(4096),
        json_mode: false,
    };

    let response = client.llm_complete(request).await?;

    #[derive(serde::Deserialize)]
    struct NewsItem {
        title: String,
        summary: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        url: String,
    }

    let json_str = extract_json(&response.content);
    let items: Vec<NewsItem> = serde_json::from_str(json_str).unwrap_or_default();

    Ok(items
        .into_iter()
        .map(|item| {
            let body = if item.body.is_empty() { item.summary.clone() } else { item.body };
            let sources = if item.url.is_empty() { vec![] } else { vec![Source { title: name.clone(), url: item.url }] };
            Article { title: item.title, summary: item.summary, body, sources, score: 0.7 }
        })
        .collect())
}

/// Generate a check-in reminder article for a contact.
async fn generate_checkin_reminder(
    client: &AppClient<tokio::io::Stdout>,
    contact: &Contact,
    style: &str,
    profile_context: &str,
) -> anyhow::Result<Article> {
    let name = contact.full_name();
    let notes_section = contact.notes.as_ref().map(|n| format!("\nNotes: {n}")).unwrap_or_default();
    let method = contact.preferred_contact_method;

    let request = LlmRequest::simple(
        format!(
            "You are a personal assistant generating a {style} check-in reminder. \
             Write a brief reminder (1-2 paragraphs) suggesting the user reach out to this contact. \
             Preferred contact method: {method}. Include a suggested message opener.{notes_section}{profile_context}"
        ),
        format!("Generate a check-in reminder for {name}."),
    );

    let response = client.llm_complete(request).await?;
    Ok(Article {
        title: format!("Time to check in with {name}"),
        summary: format!("It's been a while since you connected with {name}. Reach out via {method}."),
        body: response.content.trim().to_string(),
        sources: vec![],
        score: match contact.importance {
            Importance::VeryHigh => 0.8,
            Importance::High => 0.6,
            Importance::Medium => 0.4,
            Importance::Low => 0.3,
        },
    })
}

/// Format profile entries as context for LLM prompts.
fn format_profile_context(entries: &[ProfileEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = entries.iter().map(|e| format!("[{}] {}: {}", e.observed_at, e.category, e.content)).collect();
    format!("\n\nProfile history:\n{}", lines.join("\n"))
}

/// Search for profile facts about a contact and extract structured entries.
async fn scrape_contact_profiles(
    client: &AppClient<tokio::io::Stdout>,
    contact: &Contact,
    freshness: &str,
    max_results: usize,
) -> anyhow::Result<Vec<ProfileEntry>> {
    let query = format!("\"{}\"", contact.full_name());
    tracing::info!("Scraping profiles for: {query}");

    let args = serde_json::json!({ "query": query, "freshness": freshness });
    let results = client.call_tool("brave-search", "brave_web_search", args).await?;

    let items = parse_search_results(results);
    if items.is_empty() {
        tracing::info!("No profile results found for {}", contact.full_name());
        return Ok(vec![]);
    }

    let truncated: Vec<_> = items.into_iter().take(max_results).collect();
    let results_json = serde_json::to_string(&truncated)?;
    let name = contact.full_name();

    let request = LlmRequest {
        messages: vec![
            LlmMessage {
                role: LlmRole::System,
                content: format!(
                    "You are a personal assistant extracting profile facts about {name}. \
                     From the search results, extract factual profile entries. \
                     Classify each into exactly one category: work_history, education, achievement, news, personal. \
                     Respond with ONLY a raw JSON array: [{{\"category\": \"...\", \"content\": \"...\"}}]"
                ),
            },
            LlmMessage {
                role: LlmRole::User,
                content: format!("Extract profile facts about {name} from these search results:\n{results_json}"),
            },
        ],
        temperature: Some(0.2),
        max_completion_tokens: Some(4096),
        json_mode: false,
    };

    let response = client.llm_complete(request).await?;

    #[derive(serde::Deserialize)]
    struct ProfileFact {
        category: String,
        content: String,
    }

    let json_str = extract_json(&response.content);
    let facts: Vec<ProfileFact> = serde_json::from_str(json_str).unwrap_or_default();

    Ok(facts
        .into_iter()
        .map(|fact| new_profile_entry(&contact.id, ProfileSource::News, ProfileCategory::from_str_loose(&fact.category), fact.content))
        .collect())
}
