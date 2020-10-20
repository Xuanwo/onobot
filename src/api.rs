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
use serde::{Serialize, Deserialize};

use super::cache;
use super::config;
use telegram_bot::ParseMode::Markdown;

pub struct API {
    api: Api,
    cfg: config::Config,

    cache: RefCell<cache::Cache>,
    admins: HashSet<UserId>,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Callback {
    Offtopic {
        id: MessageId,
    }
}

impl Callback {
    fn to_string(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    fn from_string(s: &String) -> Result<Self> {
        Ok(serde_json::from_str(s.as_str())?)
    }
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
                self.handle_message(m).await?
            }
            UpdateKind::CallbackQuery(c) => {
                self.handle_callback(c).await?
            }
            _ => {}
        })
    }

    pub fn get_original_message_id(&self, m: &Message) -> Option<MessageId> {
        if m.forward.is_none() {
            return None;
        }

        let forward = m.forward.clone().unwrap();
        match forward.from {
            ForwardFrom::User { user } => {
                self.cache.borrow_mut().get(user.id, forward.date).copied()
            }
            _ => None,
        }
    }

    pub async fn handle_message(&self, m: &Message) -> Result<()> {
        match m.chat {
            MessageChat::Private(_) => {
                if m.forward.is_none() {
                    warn!("Message is not forwarded to bot, ignore this message");
                    return Ok(());
                }

                self.ask_admin(m).await?;
            }
            MessageChat::Group(_) | MessageChat::Supergroup(_) => {
                // Cache message that send to main group.
                if m.chat.id() == ChatId::from(self.cfg.main_group) {
                    self.cache.borrow_mut().set(m.from.id, m.date, m.id);
                }
            }
            _ => {}
        }


        Ok(())
    }

    pub async fn handle_callback(&self, c: &CallbackQuery) -> Result<()> {
        if c.data.is_none() {
            debug!("callback query {:?} data is empty, ignore", c.id);
            return Ok(());
        }

        match Callback::from_string(c.data.as_ref().unwrap())? {
            Callback::Offtopic { id } => {
                self.send_ot_alert(id).await?;
                self.api.send(c.acknowledge()).await?;
            }
        }

        Ok(())
    }

    pub async fn ask_admin(&self, m: &Message) -> Result<()> {
        // Check if user is an admin.
        if !self.admins.contains(&m.from.id) {
            warn!(
                "User {}({}) is not an admin",
                &m.from.first_name, &m.from.id
            );
            return Ok(());
        }

        let mut msg = m.text_reply(
            format!("该消息存在什么问题？")
        );

        let oid = self.get_original_message_id(m);
        if oid.is_none() {
            return Err(anyhow!("message id not found"));
        }

        let mut ikm = InlineKeyboardMarkup::new();
        ikm.add_row(vec![
            InlineKeyboardButton::callback("离题", Callback::Offtopic { id: oid.unwrap() }.to_string()?),
        ]);

        msg.reply_markup(ikm);
        msg.parse_mode(ParseMode::Markdown);

        self.api.send(msg).await?;

        Ok(())
    }

    pub async fn send_ot_alert(&self, original_message_id: MessageId) -> Result<()> {
        let mut msg = SendMessage::new(
            ChatId::from(self.cfg.main_group),
            format!(r#"
           请勿进行离题讨论，#archlinux-cn 仅用于 archlinux 相关话题讨论，无关主题请前往 OT 群
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
