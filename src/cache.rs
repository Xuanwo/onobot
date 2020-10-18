use log::debug;
use telegram_bot::*;

pub struct Cache(lru::LruCache<String, MessageId>);

impl Cache {
    pub fn new() -> Self {
        Self(lru::LruCache::new(1024 * 1024))
    }

    pub fn get(&mut self, user_id: UserId, time: Integer) -> Option<&MessageId> {
        let key = format!("{}/{}", user_id, time);
        debug!("cache get: {}", &key);
        self.0.get(&key)
    }

    pub fn set(&mut self, user_id: UserId, time: Integer, m: MessageId) {
        let key = format!("{}/{}", user_id, time);
        debug!("cache set: {}", &key);
        self.0.put(key, m);
    }
}
