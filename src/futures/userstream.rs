use crate::{client::*, errors::*, rest_model::*};

static LISTEN_KEY_PATH: &str = "/fapi/v1/listenKey";

#[derive(Clone)]
pub struct UserStream {
    pub client: Client,
    pub recv_window: u64,
}

impl UserStream {
    /// Get a listen key for the stream
    pub async fn start(&self) -> Result<UserDataStream> {
        self.client.post(LISTEN_KEY_PATH, None).await
    }

    /// Keep the connection alive, as the listen key becomes invalid after 60mn
    pub async fn keep_alive(&self, listen_key: &str) -> Result<Success> {
        self.client.put(LISTEN_KEY_PATH, listen_key, None).await
    }

    /// Invalidate the listen key
    pub async fn close(&self, listen_key: &str) -> Result<Success> {
        self.client.delete(LISTEN_KEY_PATH, listen_key, None).await
    }
}
