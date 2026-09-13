use super::{AppInfo, Classification, ClassificationInput, Classifier};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use tracing::debug;

const CHAT_COMPLETIONS_URL: &str = "https://api.openai.com/v1/chat/completions";

pub struct OpenAiClassifier {
    http: Client,
    api_key: String,
    model: String,
}

impl OpenAiClassifier {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            http: Client::new(),
            api_key,
            model,
        }
    }
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct Message {
    content: String,
}

#[derive(Deserialize)]
struct RawClassification {
    app: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
}

#[async_trait]
impl Classifier for OpenAiClassifier {
    async fn classify(
        &self,
        input: ClassificationInput<'_>,
        candidates: &[AppInfo],
    ) -> Result<Classification> {
        let candidate_apps: Vec<_> = candidates
            .iter()
            .map(|a| json!({"id": a.id, "description": a.description}))
            .collect();

        let system_prompt = "You are a torrent triage assistant for a home media server. \
            Given a torrent's name and category, decide which of the candidate apps should \
            handle it — read each app's description carefully. In particular, comics vs. \
            manga is decided by COUNTRY OF ORIGIN, not reading direction: identify the \
            author/creator and the work's name (and series, if recognizable) to judge \
            whether it originates from an Asian country (Japan, Korea, China, etc. — manga, \
            manhwa, manhua all count as `manga`); anything else (American, European, etc.) \
            is `comics`. `file_extensions` lists the actual file extension(s) found in the \
            downloaded content — this is a HARD FACT about format, unlike the torrent name \
            which often carries no extension at all. An audio extension (.m4b/.mp3/.m4a/\
            .flac/.aac/.ogg/.wav) means the correct app is whichever one's description \
            says AUDIO (e.g. `audiobooks`), regardless of what the title alone would \
            suggest (e.g. \"self-help book\" sounds like `novels`/`textbooks`, but if the \
            file is `.m4b` it's an audiobook). Always defer to `file_extensions` over \
            guessing format from the title. When `book_metadata` is present (from a book-database lookup keyed \
            on a guessed title, so it may be for the wrong book — sanity-check it against \
            torrent_name before trusting it), treat its `authors`/`publisher`/`categories` \
            as more reliable evidence than guessing from the torrent name alone: a publisher \
            like VIZ Media/Kodansha/Seven Seas/Yen Press or an author with a Japanese/Korean/\
            Chinese name strongly indicates `manga`, even if the torrent's own title sounds \
            Western. Also extract: (1) `title` — the series name (for comics/manga) or book \
            title (for novels/textbooks), cleaned of scene/release-group tags, resolution, \
            file format, and other noise (prefer `book_metadata.title` when it's a confident \
            match for the torrent); (2) `language` — the primary language of the actual \
            content, as an English word (e.g. \"English\", \"Japanese\", \"Spanish\"). Reply \
            with strict JSON only, no prose: {\"app\": \"<id>\", \"title\": \"<cleaned \
            title>\", \"language\": \"<language>\", \"reasoning\": \"<short reason, \
            including the country-of-origin judgment for comics/manga>\"}. If none of the \
            candidate apps clearly match, reply with app \"unknown\".";

        let user_prompt = json!({
            "torrent_name": input.torrent_name,
            "category": input.category,
            "file_extensions": input.file_extensions,
            "book_metadata": input.book_metadata.map(|m| json!({
                "title": m.title,
                "authors": m.authors,
                "publisher": m.publisher,
                "description": m.description,
                "categories": m.categories,
                "language": m.language,
            })),
            "candidate_apps": candidate_apps,
        })
        .to_string();

        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt},
            ],
            "response_format": {"type": "json_object"},
            "temperature": 0,
        });

        let resp = self
            .http
            .post(CHAT_COMPLETIONS_URL)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("OpenAI request failed")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("OpenAI API error ({status}): {text}");
        }

        let parsed: ChatResponse =
            serde_json::from_str(&text).context("parse OpenAI response envelope")?;
        let content = parsed
            .choices
            .into_iter()
            .next()
            .context("OpenAI response had no choices")?
            .message
            .content;

        let raw: RawClassification =
            serde_json::from_str(&content).context("parse OpenAI classification JSON")?;
        debug!(app = %raw.app, title = ?raw.title, language = ?raw.language, "classified torrent");

        Ok(Classification {
            app_id: raw.app,
            title: raw.title,
            language: raw.language,
            reasoning: raw.reasoning,
        })
    }
}
