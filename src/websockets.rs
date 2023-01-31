use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use futures::{Future, SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::from_str;
use tokio::{net::TcpStream, task::JoinHandle};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{handshake::client::Response, Message},
    MaybeTlsStream, WebSocketStream,
};
use url::Url;

use crate::{config::Config, errors::*};

pub static STREAM_ENDPOINT: &str = "stream";
pub static WS_ENDPOINT: &str = "ws";
pub static OUTBOUND_ACCOUNT_INFO: &str = "outboundAccountInfo";
pub static OUTBOUND_ACCOUNT_POSITION: &str = "outboundAccountPosition";
pub static EXECUTION_REPORT: &str = "executionReport";
pub static KLINE: &str = "kline";
pub static AGGREGATED_TRADE: &str = "aggTrade";
pub static DEPTH_ORDERBOOK: &str = "depthUpdate";
pub static PARTIAL_ORDERBOOK: &str = "lastUpdateId";
pub static DAYTICKER: &str = "24hrTicker";

pub fn all_ticker_stream() -> &'static str {
    "!ticker@arr"
}

pub fn ticker_stream(symbol: &str) -> String {
    format!("{symbol}@ticker")
}

pub fn agg_trade_stream(symbol: &str) -> String {
    format!("{symbol}@aggTrade")
}

pub fn trade_stream(symbol: &str) -> String {
    format!("{symbol}@trade")
}

pub fn kline_stream(symbol: &str, interval: &str) -> String {
    format!("{symbol}@kline_{interval}")
}

pub fn book_ticker_stream(symbol: &str) -> String {
    format!("{symbol}@bookTicker")
}

pub fn all_book_ticker_stream() -> &'static str {
    "!bookTicker"
}

pub fn all_mini_ticker_stream() -> &'static str {
    "!miniTicker@arr"
}

pub fn mini_ticker_stream(symbol: &str) -> String {
    format!("{symbol}@miniTicker")
}

/// # Arguments
///
/// * `symbol`: the market symbol
/// * `levels`: 5, 10 or 20
/// * `update_speed`: 1000 or 100
pub fn partial_book_depth_stream(symbol: &str, levels: u16, update_speed: u16) -> String {
    format!("{}@depth{}@{}ms", symbol, levels, update_speed)
}

/// # Arguments
///
/// * `symbol`: the market symbol
/// * `update_speed`: 1000 or 100
pub fn diff_book_depth_stream(symbol: &str, update_speed: u16) -> String {
    format!("{symbol}@depth@{update_speed}ms")
}

fn combined_stream(streams: Vec<String>) -> String {
    streams.join("/")
}

pub struct WebSockets<'a, WE> {
    pub socket: Option<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response)>,
    handler: Box<dyn FnMut(WE) -> Result<()> + 'a + Send>,
    conf: Config,
}

impl<'a, WE: serde::de::DeserializeOwned> WebSockets<'a, WE> {
    /// New websocket holder with default configuration
    /// # Examples
    /// see examples/binance_websockets.rs
    pub fn new<Callback>(handler: Callback) -> WebSockets<'a, WE>
    where
        Callback: FnMut(WE) -> Result<()> + 'a + Send,
    {
        Self::new_with_options(handler, Config::default())
    }

    /// New websocket holder with provided configuration
    /// # Examples
    /// see examples/binance_websockets.rs
    pub fn new_with_options<Callback>(handler: Callback, conf: Config) -> WebSockets<'a, WE>
    where
        Callback: FnMut(WE) -> Result<()> + 'a + Send,
    {
        WebSockets {
            socket: None,
            handler: Box::new(handler),
            conf,
        }
    }

    /// Connect to multiple websocket endpoints
    /// N.B: WE has to be CombinedStreamEvent
    pub async fn connect_multiple(&mut self, endpoints: Vec<String>) -> Result<()> {
        let mut url = Url::parse(&self.conf.ws_endpoint)?;
        url.path_segments_mut()
            .map_err(|_| Error::UrlParserError(url::ParseError::RelativeUrlWithoutBase))?
            .push(STREAM_ENDPOINT);
        url.set_query(Some(&format!("streams={}", combined_stream(endpoints))));

        match connect_async(url).await {
            Ok(answer) => {
                self.socket = Some(answer);
                Ok(())
            }
            Err(e) => Err(Error::Msg(format!("Error during handshake {}", e))),
        }
    }

    /// Connect to a websocket endpoint
    pub async fn connect(&mut self, endpoint: &str) -> Result<()> {
        let wss: String = format!("{}/{}/{}", self.conf.ws_endpoint, WS_ENDPOINT, endpoint);
        let url = Url::parse(&wss)?;

        match connect_async(url).await {
            Ok(answer) => {
                self.socket = Some(answer);

                // TODO: add keepalive
                Ok(())
            }
            Err(e) => Err(Error::Msg(format!("Error during handshake {}", e))),
        }
    }

    /// Disconnect from the endpoint
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(ref mut socket) = self.socket {
            socket.0.close(None).await?;
            Ok(())
        } else {
            Err(Error::Msg("Not able to close the connection".to_string()))
        }
    }

    pub fn socket(&self) -> &Option<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response)> {
        &self.socket
    }

    pub async fn event_loop(&mut self, running: &AtomicBool) -> Result<()> {
        while running.load(Ordering::Relaxed) {
            if let Some((ws, _)) = self.socket.as_mut() {
                let message = ws
                    .next()
                    .await
                    .ok_or_else(|| Error::Msg("Disconnected".to_string()))
                    .and_then(|r| Ok(r?));

                match message {
                    Ok(Message::Text(msg)) => {
                        if msg.is_empty() {
                            // return Ok(());
                        }
                        let event_res: std::result::Result<WE, _> = from_str(msg.as_str());
                        match event_res {
                            Ok(event) => {
                                (self.handler)(event)?;
                            }
                            Err(_) => {
                                // return Err(Error::Msg(format!("Error during parsing {e}")));
                            }
                        }
                    }
                    // Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
                    Ok(Message::Close(e)) => {
                        return Err(Error::Msg(format!("Disconnected {e:?}")));
                    }
                    Ok(_) => {}
                    Err(e) => {
                        return Err(Error::Msg(format!("Error during message {e}")));
                    }
                }
            }
        }
        Ok(())
    }
}

pub struct WebsocketImproved {
    pub socket: (WebSocketStream<MaybeTlsStream<TcpStream>>, Response),
    conf: Config,
}

impl WebsocketImproved {
    /// Connect to a websocket endpoint
    pub async fn connect(listen_key: &str, conf: Option<Config>) -> Result<Self> {
        let conf = conf.unwrap_or_default();
        let wss = format!("{}/{}/{}", conf.ws_endpoint, WS_ENDPOINT, listen_key);
        let url = Url::parse(&wss)?;

        Ok(connect_async(url).await.map(|socket| Self { socket, conf })?)
    }

    /// Connect to a futures websocket endpoint
    pub async fn connect_futures(listen_key: &str, conf: Option<Config>) -> Result<Self> {
        let conf = conf.unwrap_or_default();
        let wss = format!("{}/{}/{}", conf.futures_ws_endpoint, WS_ENDPOINT, listen_key);
        let url = Url::parse(&wss)?;

        Ok(connect_async(url).await.map(|socket| Self { socket, conf })?)
    }

    /// If the callback returns an error, the stream will be stopped
    pub fn start_streaming_async<WsItemT, F, E, T, ParamT>(
        self,
        callback: F,
        param: ParamT,
        should_stop: Arc<AtomicBool>,
    ) -> Result<JoinHandle<()>>
    where
        WsItemT: DeserializeOwned + Send,
        F: Fn(Result<WsItemT>, ParamT) -> T + Send + Sync + 'static,
        T: Future<Output = std::result::Result<(), E>> + Send,
        ParamT: Send + Sync + Clone + 'static,
    {
        let (ws, _) = self.socket;

        let (mut ws_write, ws_read) = ws.split();

        let handle = tokio::task::spawn(async move {
            let mut stream = ws_read; //.map_err(|e| Error::Msg(format!("Error {e}")));

            while let Some(msg) = stream.next().await {
                // TODO: this will wait for the stream to get a message, we should refactor this
                if should_stop.load(Ordering::Relaxed) {
                    break;
                }
                match msg {
                    Ok(Message::Text(msg)) => {
                        let deserialize_res = from_str::<WsItemT>(&msg).map_err(Into::into);
                        if let Err(_) = callback(deserialize_res, param.clone()).await {
                            // if the callback returns an error, we should stop the loop
                            should_stop.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                    Ok(Message::Close(_maybe_frame)) => {
                        should_stop.store(true, Ordering::Relaxed);
                        break;
                    }
                    Ok(Message::Ping(v)) => {
                        let _res = ws_write.send(Message::Pong(v)).await;
                    }
                    Ok(m) => {
                        println!("unexpected message {m:?}");
                    }

                    Err(e) => {
                        println!("error {e}");
                        should_stop.store(true, Ordering::Relaxed);
                    }
                }
            }
        });

        Ok(handle)
    }
}
