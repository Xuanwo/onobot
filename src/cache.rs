use log::debug;
use telegram_bot::*;
use anyhow::Result;

pub struct Cache(sled::Db);

impl Cache {
    pub fn new<P: AsRef<std::path::Path>>(path: P) -> Result<Cache> {
        let db = sled::open(path)?;
        Ok(Self(db))
    }

    pub fn get(&mut self, time: Integer, user_name: String) -> Option<MessageId> {
        let key = format!("{}/{}", time, user_name);
        let value = self.0.get(&key).expect("read from cache failed");
        if value.is_none() {
            debug!("cache not exist: {}", &key);
            return None;
        }
        let id: i64 = bincode::deserialize(&value.unwrap().to_vec()).expect("invalid value");
        debug!("cache get: {}, {}", &key, id);

        Some(MessageId::from(id))
    }

    pub fn set(&mut self, time: Integer, user_name: String, m: MessageId) {
        // TODO: remove old messages.
        let key = format!("{}/{}", time, user_name);
        debug!("cache set: {}, {}", &key, &m);
        self.0.insert(
            &key,
            bincode::serialize(&m).expect("bincode serialize failed"),
        ).expect("write into cache failed");
    }
}
