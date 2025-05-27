use crate::{
    prelude::*,
    ws::message_types::{AllMids, Candle, L2Book, OrderUpdates, Trades, User},
    ActiveAssetCtx, Error, Notification, UserFills, UserFundings, UserNonFundingLedgerUpdates,
    WebData2,
};
use futures_util::stream::SplitStream;
use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use log::{error, warn};
use serde::{Deserialize, Serialize};
use std::{
    borrow::BorrowMut,
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    net::TcpStream,
    spawn,
    sync::{mpsc::UnboundedSender, Mutex},
    time,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{self, protocol},
    MaybeTlsStream, WebSocketStream,
};

use ethers::types::H160;

#[allow(dead_code)]
#[derive(Debug)]
struct SubscriptionData {
    sending_channel: UnboundedSender<Message>,
    subscription_id: u32,
    id: String,
}

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct WsManager {
    stop_flag: Arc<AtomicBool>,
    writer: Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, protocol::Message>>>,
    reader: Arc<Mutex<SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>>,

    // subscriptions
    subscriptions: Arc<Mutex<HashMap<String, Vec<SubscriptionData>>>>,
    subscription_id: u32,
    subscription_identifiers: HashMap<u32, String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum Subscription {
    AllMids,
    Notification { user: H160 },
    WebData2 { user: H160 },
    Candle { coin: String, interval: String },
    L2Book { coin: String },
    Trades { coin: String },
    OrderUpdates { user: H160 },
    UserEvents { user: H160 },
    UserFills { user: H160 },
    UserFundings { user: H160 },
    UserNonFundingLedgerUpdates { user: H160 },
    ActiveAssetCtx { coin: String },
}

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "channel")]
#[serde(rename_all = "camelCase")]
pub enum Message {
    NoData,
    HyperliquidError(String),
    AllMids(AllMids),
    Trades(Trades),
    L2Book(L2Book),
    User(User),
    UserFills(UserFills),
    Candle(Candle),
    SubscriptionResponse,
    OrderUpdates(OrderUpdates),
    UserFundings(UserFundings),
    UserNonFundingLedgerUpdates(UserNonFundingLedgerUpdates),
    Notification(Notification),
    WebData2(WebData2),
    ActiveAssetCtx(ActiveAssetCtx),
    Pong,
}

#[derive(Serialize)]
pub(crate) struct SubscriptionSendData<'a> {
    method: &'static str,
    subscription: &'a serde_json::Value,
}

#[derive(Serialize)]
pub(crate) struct Ping {
    method: &'static str,
}

impl WsManager {
    const SEND_PING_INTERVAL: u64 = 15;

    pub(crate) async fn new(url: String, reconnect: bool) -> Result<WsManager> {
        let stop_flag = Arc::new(AtomicBool::new(false));

        let ws_stream = Self::connect(&url).await?;
        let (writer_split, reader_split) = ws_stream.split();
        let writer = Arc::new(Mutex::new(writer_split));
        let reader = Arc::new(Mutex::new(reader_split));

        let subscriptions = Arc::new(Mutex::new(HashMap::new()));

        let reader_clone = Arc::clone(&reader);
        let writer_clone = Arc::clone(&writer);
        let subscriptions_clone = Arc::clone(&subscriptions);
        let stop_flag_clone = Arc::clone(&stop_flag);
        let url_clone = url.clone();

        spawn(async move {
            loop {
                if stop_flag_clone.load(Ordering::Relaxed) {
                    break;
                }

                let mut reader_lock = reader_clone.lock().await;
                match reader_lock.next().await {
                    Some(Ok(msg)) => {
                        if let Err(err) =
                            WsManager::parse_and_send_data(Ok(msg), &subscriptions_clone).await
                        {
                            error!("Error processing message: {err}");
                        }
                    }
                    Some(Err(err)) => {
                        error!("WebSocket error: {err}");
                        if reconnect {
                            warn!("Attempting reconnect after error...");
                            time::sleep(Duration::from_secs(1)).await;
                            match Self::connect(&url_clone).await {
                                Ok(new_ws) => {
                                    let (new_writer, new_reader) = new_ws.split();
                                    *writer_clone.lock().await = new_writer;
                                    *reader_clone.lock().await = new_reader;
                                }
                                Err(e) => {
                                    error!("Reconnect failed: {e}");
                                }
                            }
                        } else {
                            break;
                        }
                    }
                    None => {
                        warn!("WebSocket connection closed.");
                        break;
                    }
                }
            }
        });

        let ping_writer = Arc::clone(&writer);
        let stop_flag_ping = Arc::clone(&stop_flag);
        spawn(async move {
            while !stop_flag_ping.load(Ordering::Relaxed) {
                match serde_json::to_string(&Ping { method: "ping" }) {
                    Ok(payload) => {
                        let mut writer = ping_writer.lock().await;
                        if let Err(err) = writer.send(protocol::Message::Text(payload.into())).await
                        {
                            error!("Error pinging server: {err}");
                            break; // остановим пинг на ошибке
                        }
                    }
                    Err(err) => {
                        error!("Failed to serialize ping: {err}");
                    }
                }
                time::sleep(Duration::from_secs(Self::SEND_PING_INTERVAL)).await;
            }
        });

        Ok(WsManager {
            stop_flag,
            writer,
            reader,
            subscriptions,
            subscription_id: 0,
            subscription_identifiers: HashMap::new(),
        })
    }

    async fn connect(url: &str) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>> {
        let (ws_stream, _) = connect_async(url)
            .await
            .map_err(|e| Error::Websocket(e.to_string()))?;

        if let Some(tcp_stream) = Self::get_tcp_stream(&ws_stream) {
            tcp_stream
                .set_nodelay(true)
                .map_err(|e| Error::Websocket(format!("Failed to set TCP_NODELAY: {e}")))?;
        }
        Ok(ws_stream)
    }

    fn get_tcp_stream(
        ws_stream: &WebSocketStream<MaybeTlsStream<TcpStream>>,
    ) -> Option<&TcpStream> {
        match ws_stream.get_ref() {
            MaybeTlsStream::Plain(tcp_stream) => Some(tcp_stream),
            _ => None, // TLS not supported in this case
        }
    }

    // fn get_tcp_stream(
    //     ws_stream: &WebSocketStream<MaybeTlsStream<TcpStream>>,
    // ) -> Option<&TcpStream> {
    //     match ws_stream.get_ref() {
    //         MaybeTlsStream::Plain(tcp_stream) => Some(tcp_stream),
    //         MaybeTlsStream::Rustls(stream) => Some(stream.get_ref().0),
    //         MaybeTlsStream::NativeTls(stream) => stream.get_ref().downcast_ref::<TcpStream>(),
    //     }
    // }

    fn get_identifier(message: &Message) -> Result<String> {
        match message {
            Message::AllMids(_) => serde_json::to_string(&Subscription::AllMids)
                .map_err(|e| Error::JsonParse(e.to_string())),
            Message::User(_) => Ok("userEvents".to_string()),
            Message::UserFills(fills) => serde_json::to_string(&Subscription::UserFills {
                user: fills.data.user,
            })
            .map_err(|e| Error::JsonParse(e.to_string())),
            Message::Trades(trades) => {
                if trades.data.is_empty() {
                    Ok(String::default())
                } else {
                    serde_json::to_string(&Subscription::Trades {
                        coin: trades.data[0].coin.clone(),
                    })
                    .map_err(|e| Error::JsonParse(e.to_string()))
                }
            }
            Message::L2Book(l2_book) => serde_json::to_string(&Subscription::L2Book {
                coin: l2_book.data.coin.clone(),
            })
            .map_err(|e| Error::JsonParse(e.to_string())),
            Message::Candle(candle) => serde_json::to_string(&Subscription::Candle {
                coin: candle.data.coin.clone(),
                interval: candle.data.interval.clone(),
            })
            .map_err(|e| Error::JsonParse(e.to_string())),
            Message::OrderUpdates(_) => Ok("orderUpdates".to_string()),
            Message::UserFundings(fundings) => serde_json::to_string(&Subscription::UserFundings {
                user: fundings.data.user,
            })
            .map_err(|e| Error::JsonParse(e.to_string())),
            Message::UserNonFundingLedgerUpdates(user_non_funding_ledger_updates) => {
                serde_json::to_string(&Subscription::UserNonFundingLedgerUpdates {
                    user: user_non_funding_ledger_updates.data.user,
                })
                .map_err(|e| Error::JsonParse(e.to_string()))
            }
            Message::Notification(_) => Ok("notification".to_string()),
            Message::WebData2(web_data2) => serde_json::to_string(&Subscription::WebData2 {
                user: web_data2.data.user,
            })
            .map_err(|e| Error::JsonParse(e.to_string())),
            Message::ActiveAssetCtx(active_asset_ctx) => {
                serde_json::to_string(&Subscription::ActiveAssetCtx {
                    coin: active_asset_ctx.data.coin.clone(),
                })
                .map_err(|e| Error::JsonParse(e.to_string()))
            }
            Message::SubscriptionResponse | Message::Pong => Ok(String::default()),
            Message::NoData => Ok("".to_string()),
            Message::HyperliquidError(err) => Ok(format!("hyperliquid error: {err:?}")),
        }
    }

    async fn parse_and_send_data(
        data: std::result::Result<protocol::Message, tungstenite::Error>,
        subscriptions: &Arc<Mutex<HashMap<String, Vec<SubscriptionData>>>>,
    ) -> Result<()> {
        match data {
            Ok(data) => match data.into_text() {
                Ok(data) => {
                    if !data.starts_with('{') {
                        return Ok(());
                    }
                    let message = serde_json::from_str::<Message>(&data)
                        .map_err(|e| Error::JsonParse(e.to_string()))?;
                    let identifier = WsManager::get_identifier(&message)?;
                    if identifier.is_empty() {
                        return Ok(());
                    }

                    let mut subscriptions = subscriptions.lock().await;
                    let mut res = Ok(());
                    if let Some(subscription_datas) = subscriptions.get_mut(&identifier) {
                        for subscription_data in subscription_datas {
                            if let Err(e) = subscription_data
                                .sending_channel
                                .send(message.clone())
                                .map_err(|e| Error::WsSend(e.to_string()))
                            {
                                res = Err(e);
                            }
                        }
                    }
                    res
                }
                Err(err) => {
                    let error = Error::ReaderTextConversion(err.to_string());
                    Ok(WsManager::send_to_all_subscriptions(
                        subscriptions,
                        Message::HyperliquidError(error.to_string()),
                    )
                    .await?)
                }
            },
            Err(err) => {
                let error = Error::GenericReader(err.to_string());
                Ok(WsManager::send_to_all_subscriptions(
                    subscriptions,
                    Message::HyperliquidError(error.to_string()),
                )
                .await?)
            }
        }
    }

    async fn send_to_all_subscriptions(
        subscriptions: &Arc<Mutex<HashMap<String, Vec<SubscriptionData>>>>,
        message: Message,
    ) -> Result<()> {
        let mut subscriptions = subscriptions.lock().await;
        let mut res = Ok(());
        for subscription_datas in subscriptions.values_mut() {
            for subscription_data in subscription_datas {
                if let Err(e) = subscription_data
                    .sending_channel
                    .send(message.clone())
                    .map_err(|e| Error::WsSend(e.to_string()))
                {
                    res = Err(e);
                }
            }
        }
        res
    }

    async fn send_subscription_data(
        method: &'static str,
        writer: &mut SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, protocol::Message>,
        identifier: &str,
    ) -> Result<()> {
        let payload = serde_json::to_string(&SubscriptionSendData {
            method,
            subscription: &serde_json::from_str::<serde_json::Value>(identifier)
                .map_err(|e| Error::JsonParse(e.to_string()))?,
        })
        .map_err(|e| Error::JsonParse(e.to_string()))?;

        writer
            .send(protocol::Message::Text(payload.into()))
            .await
            .map_err(|e| Error::Websocket(e.to_string()))?;
        Ok(())
    }

    async fn subscribe(
        writer: &mut SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, protocol::Message>,
        identifier: &str,
    ) -> Result<()> {
        Self::send_subscription_data("subscribe", writer, identifier).await
    }

    async fn unsubscribe(
        writer: &mut SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, protocol::Message>,
        identifier: &str,
    ) -> Result<()> {
        Self::send_subscription_data("unsubscribe", writer, identifier).await
    }

    pub(crate) async fn add_subscription(
        &mut self,
        identifier: String,
        sending_channel: UnboundedSender<Message>,
    ) -> Result<u32> {
        let mut subscriptions = self.subscriptions.lock().await;

        let identifier_entry = if let Subscription::UserEvents { user: _ } =
            serde_json::from_str::<Subscription>(&identifier)
                .map_err(|e| Error::JsonParse(e.to_string()))?
        {
            "userEvents".to_string()
        } else if let Subscription::OrderUpdates { user: _ } =
            serde_json::from_str::<Subscription>(&identifier)
                .map_err(|e| Error::JsonParse(e.to_string()))?
        {
            "orderUpdates".to_string()
        } else {
            identifier.clone()
        };
        let subscriptions = subscriptions
            .entry(identifier_entry.clone())
            .or_insert(Vec::new());

        if !subscriptions.is_empty() && identifier_entry.eq("userEvents") {
            return Err(Error::UserEvents);
        }

        if subscriptions.is_empty() {
            Self::subscribe(self.writer.lock().await.borrow_mut(), identifier.as_str()).await?;
        }

        let subscription_id = self.subscription_id;
        self.subscription_identifiers
            .insert(subscription_id, identifier.clone());
        subscriptions.push(SubscriptionData {
            sending_channel,
            subscription_id,
            id: identifier,
        });

        self.subscription_id += 1;
        Ok(subscription_id)
    }

    pub(crate) async fn remove_subscription(&mut self, subscription_id: u32) -> Result<()> {
        let identifier = self
            .subscription_identifiers
            .get(&subscription_id)
            .ok_or(Error::SubscriptionNotFound)?
            .clone();

        let identifier_entry = if let Subscription::UserEvents { user: _ } =
            serde_json::from_str::<Subscription>(&identifier)
                .map_err(|e| Error::JsonParse(e.to_string()))?
        {
            "userEvents".to_string()
        } else if let Subscription::OrderUpdates { user: _ } =
            serde_json::from_str::<Subscription>(&identifier)
                .map_err(|e| Error::JsonParse(e.to_string()))?
        {
            "orderUpdates".to_string()
        } else {
            identifier.clone()
        };

        self.subscription_identifiers.remove(&subscription_id);

        let mut subscriptions = self.subscriptions.lock().await;

        let subscriptions = subscriptions
            .get_mut(&identifier_entry)
            .ok_or(Error::SubscriptionNotFound)?;
        let index = subscriptions
            .iter()
            .position(|subscription_data| subscription_data.subscription_id == subscription_id)
            .ok_or(Error::SubscriptionNotFound)?;
        subscriptions.remove(index);

        if subscriptions.is_empty() {
            Self::unsubscribe(self.writer.lock().await.borrow_mut(), identifier.as_str()).await?;
        }
        Ok(())
    }
}

impl Drop for WsManager {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}
