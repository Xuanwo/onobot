use std::cell::RefCell;
use std::collections::HashSet;
use std::env;
use std::io::Write;

use anyhow::{anyhow, Result};
use futures::StreamExt;
use hyper::client::HttpConnector;
use hyper::service::service_fn;
use hyper::Client;
use hyper_proxy::{Intercept, Proxy, ProxyConnector};
use log::{debug, error, info, warn};
use telegram_bot::connector::default_connector;
use telegram_bot::connector::hyper::HyperConnector;
use telegram_bot::MessageEntityKind::BotCommand;
use telegram_bot::*;

use super::cache;
use super::config;
use telegram_bot::ParseMode::Markdown;

pub struct API {
    api: Api,
    cfg: config::Config,

    cache: RefCell<cache::Cache>,
    admins: HashSet<UserId>,
}

impl API {
    pub async fn new(cfg: config::Config) -> Result<API> {
        let token = &cfg.token;

        let connector = if env::var("https_proxy").is_ok() {
            let proxy_uri = env::var("https_proxy")?.parse().unwrap();
            let mut proxy = Proxy::new(Intercept::All, proxy_uri);
            let connector = HttpConnector::new();
            let proxy_connector = ProxyConnector::from_proxy(connector, proxy).unwrap();
            Box::new(HyperConnector::new(
                Client::builder().build(proxy_connector),
            ))
        } else {
            default_connector()
        };

        let api = Api::with_connector(token, connector);

        // Fetch admins.
        let mut h = HashSet::new();
        match api
            .send(GetChatAdministrators::new(ChatId::from(cfg.main_group)))
            .await
        {
            Err(err) => error!("get chat administrator: {}", err.to_string()),
            Ok(admins) => {
                for m in admins.iter() {
                    h.insert(m.user.id);
                }
            }
        }

        Ok(Self {
            api,
            cfg,
            cache: RefCell::new(cache::Cache::new()),
            admins: h,
        })
    }

    pub async fn run(&self) -> Result<()> {
        let mut stream = self.api.stream();

        while let Some(update) = stream.next().await {
            match update {
                Err(err) => error!("fetch update: {}", err),
                Ok(update) => match self.handle(&update).await {
                    Ok(_) => info!("message {} handled correctly.", &update.id),
                    Err(err) => error!("handle update {}: {}", &update.id, err),
                },
            }
        }

        Ok(())
    }

    // We only handle following situation:
    //   - User is a admin
    //   - Message is forwarded to bot private chat
    //   - Bot is mentioned at admin group
    pub async fn handle(&self, u: &Update) -> Result<()> {
        debug!("{:?}", &u);

        Ok(match &u.kind {
            UpdateKind::Message(m) => {
                if m.chat.id() == ChatId::from(self.cfg.main_group) {
                    self.cache.borrow_mut().set(m.from.id, m.date, m.id);
                }

                self.handle_message(m).await?
            }
            _ => {}
        })
    }

    pub fn get_original_message_id(&self, m: &Message) -> Option<MessageId> {
        match &m.chat {
            MessageChat::Private(u) => {
                if m.forward.is_none() {
                    return None;
                }

                let forward = m.forward.clone().unwrap();
                return match forward.from {
                    ForwardFrom::User { user } => {
                        self.cache.borrow_mut().get(user.id, forward.date).copied()
                    }
                    _ => None,
                };
            }
            MessageChat::Group(_) | MessageChat::Supergroup(_) => {
                if m.chat.id() != ChatId::from(self.cfg.admin_group) {
                    return None;
                }
                if m.reply_to_message.is_none() {
                    return None;
                }
                return match m.reply_to_message.as_ref().unwrap().as_ref() {
                    MessageOrChannelPost::Message(m) => Some(m.id),
                    MessageOrChannelPost::ChannelPost(_) => None,
                };
            }
            _ => error!("invalid chat type: {:?}", &m.chat),
        }

        None
    }

    pub async fn handle_message(&self, m: &Message) -> Result<()> {
        // Check if user is an admin.
        if !self.admins.contains(&m.from.id) {
            warn!(
                "User {}({}) is not an admin",
                &m.from.first_name, &m.from.id
            );
            return Ok(());
        }

        let original_message_id = self.get_original_message_id(m);
        if original_message_id.is_none() {
            warn!("Message original can't find, ignore this message");
            return Ok(());
        }

        self.send_ot_alert(original_message_id.unwrap(), m).await?;

        info!("管理员 @{} ({}) 出警成功", &m.from.first_name, &m.from.id);
        self.api
            .send(SendMessage::new(
                m.chat.id(),
                format!("管理员 @{} ({}) 出警成功", m.from.first_name, m.from.id),
            ))
            .await?;

        Ok(())
    }

    pub async fn send_ot_alert(&self, original_message_id: MessageId, m: &Message) -> Result<()> {
        let mut msg = SendMessage::new(
            ChatId::from(self.cfg.main_group),
            format!(r#"
            此话题已经偏离本群主题，请移步至相应的讨论群继续话题
            "#),
        );

        let mut ikm = InlineKeyboardMarkup::new();
        // Add button for ot group
        ikm.add_row(vec![
            InlineKeyboardButton::url("跳转到 OT 群", &self.cfg.offtopic_group),
            InlineKeyboardButton::url("申诉", &self.cfg.meta_group),
        ]);

        msg.reply_markup(ikm);
        msg.reply_to(original_message_id);
        msg.parse_mode(ParseMode::Markdown);

        self.api.send(msg).await?;

        Ok(())
    }
}
