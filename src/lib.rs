mod http_api {
    use anyhow::{anyhow, Error};

    const API_ROOM_INIT: &str = "http://api.live.bilibili.com/room/v1/Room/room_init?id=";

    pub async fn get_room_id(id: u32) -> Result<u32, Error> {
        let client = hyper::Client::new();
        let uri = format!("{}{}", API_ROOM_INIT, id).parse().unwrap();
        let resp = client.get(uri).await?;
        let bytes = hyper::body::to_bytes(resp).await?;
        let str = unsafe { std::str::from_utf8_unchecked(&bytes) };
        let json = json::parse(str).unwrap();
        if json["code"] != 0 {
            return Err(anyhow!(
                "Bilibili API error: {}",
                json["msg"].as_str().unwrap()
            ));
        }
        Ok(json["data"]["room_id"].as_u32().unwrap())
    }
}

pub mod chat {
    use super::msg::Message;
    use anyhow::{anyhow, Error};
    use bytes::{Buf, BufMut, BytesMut};
    use futures_sink::Sink;
    use futures_util::future;
    use futures_util::{sink::SinkExt, stream::StreamExt};
    use std::fmt::Write;
    use std::future::Future;
    use std::io::Cursor;
    use tokio::io;
    use tokio::net::TcpStream;
    use tokio::time::{self, Duration};
    use tokio_util::codec::{Decoder, Encoder, FramedRead, FramedWrite};

    const ADDR: (&str, u16) = ("broadcastlv.chat.bilibili.com", 2243);

    const HEARTBEAT_DELAY: Duration = Duration::from_secs(30);

    const HEADER_LENGTH: usize = 16;

    const PROTO_HEARTBEAT: u16 = 1;

    const OP_HEARTBEAT: u32 = 2;
    const OP_HEARTBEAT_REPLY: u32 = 3;
    const OP_MESSAGE: u32 = 5;
    const OP_USER_AUTHENTICATION: u32 = 7;
    const OP_CONNECT_SUCCESS: u32 = 8;

    const SEQUENCE_ID_DEFAULT: u32 = 1;

    pub async fn connect<F, Fut>(id: u32, handle_packet: F) -> Result<(), Error>
    where
        F: FnMut(ChatPacket) -> Fut,
        Fut: Future<Output = ()>,
    {
        let id = super::http_api::get_room_id(id).await?;
        let mut stream = TcpStream::connect(ADDR).await?;
        let (r, w) = TcpStream::split(&mut stream);
        let r = FramedRead::new(r, ChatCodec);
        let mut w = FramedWrite::new(w, ChatCodec);

        w.send(RawChatPacket::authenticate(id)).await?;
        future::try_join(handle_stream(r, handle_packet), send_heartbeat(w)).await?;

        Ok(())
    }

    async fn handle_stream<F, Fut>(
        mut stream: impl StreamExt<Item = Result<ChatPacket, Error>> + Unpin,
        mut handle_packet: F,
    ) -> Result<(), Error>
    where
        F: FnMut(ChatPacket) -> Fut,
        Fut: Future<Output = ()>,
    {
        loop {
            match stream.next().await {
                Some(res) => handle_packet(res?).await,
                None => break,
            }
        }
        Ok(())
    }

    async fn send_heartbeat(
        mut sink: impl Sink<RawChatPacket, Error = io::Error> + Unpin,
    ) -> Result<(), Error> {
        loop {
            sink.send(RawChatPacket::heartbeat()).await?;
            time::delay_for(HEARTBEAT_DELAY).await;
        }
    }

    pub enum ChatPacket {
        ConnectSuccess,
        Popularity(u32),
        Message(Message),
    }

    struct RawChatPacket {
        proto_ver: u16,
        operation: u32,
        payload: Vec<u8>,
    }

    impl RawChatPacket {
        fn authenticate(room_id: u32) -> Self {
            let mut payload = String::new();
            write!(payload, r#"{{"uid":0,"roomid":{}}}"#, room_id).unwrap();
            Self {
                proto_ver: PROTO_HEARTBEAT,
                operation: OP_USER_AUTHENTICATION,
                payload: payload.into_bytes(),
            }
        }

        fn heartbeat() -> Self {
            Self {
                proto_ver: PROTO_HEARTBEAT,
                operation: OP_HEARTBEAT,
                payload: Vec::new(),
            }
        }
    }

    /// Codec for chat packets
    ///
    /// packet length: u32
    /// header length: u16 (16)
    /// protocol version: u16
    /// operation: u32
    /// sequence: u32 (1)
    /// data: [u8]
    struct ChatCodec;

    impl Encoder<RawChatPacket> for ChatCodec {
        type Error = io::Error;

        fn encode(&mut self, pk: RawChatPacket, dst: &mut BytesMut) -> Result<(), Self::Error> {
            let len = HEADER_LENGTH + pk.payload.len();
            dst.reserve(len);
            dst.put_u32(len as u32);
            dst.put_u16(HEADER_LENGTH as u16);
            dst.put_u16(pk.proto_ver);
            dst.put_u32(pk.operation);
            dst.put_u32(SEQUENCE_ID_DEFAULT);
            dst.put(&pk.payload[..]);
            Ok(())
        }
    }

    impl Decoder for ChatCodec {
        type Item = ChatPacket;
        type Error = Error;

        fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
            let src_len = src.len();
            if src_len < 4 {
                return Ok(None);
            }
            let len = Cursor::new(&src).get_u32() as usize;
            if src_len < len {
                // Reserved bytes counts from the cursor index
                src.reserve(len);
                return Ok(None);
            }
            src.advance(8); // packet length + header length + protocol version
            let operation = src.get_u32();
            src.advance(4); // Sequence ID

            let len = len - HEADER_LENGTH;
            Ok(Some(match operation {
                OP_CONNECT_SUCCESS => {
                    let payload = &src[0..len];
                    if payload != b"{\"code\":0}" {
                        let str = unsafe { std::str::from_utf8_unchecked(&src[0..len]) };
                        return Err(anyhow!("connection failed: {}", str));
                    }
                    src.advance(len);
                    ChatPacket::ConnectSuccess
                }
                OP_HEARTBEAT_REPLY => ChatPacket::Popularity(src.get_u32()),
                OP_MESSAGE => {
                    let str = unsafe { std::str::from_utf8_unchecked(&src[0..len]) };
                    let json = json::parse(&str).unwrap();
                    let msg = super::msg::Message::parse(json)
                        .unwrap_or_else(|| Message::ParsingError(str.to_owned()));
                    src.advance(len);
                    ChatPacket::Message(msg)
                }
                _ => {
                    // Unexpected opreation, skip this packet
                    src.advance(len);
                    return Ok(None);
                }
            }))
        }
    }
}

pub mod msg {

    use self::Message::*;
    use std::fmt;

    pub enum Message {
        Preparing,
        Live,
        RoomChange {
            title: String,
            area_name: String,
            parent_area_name: String,
        },
        Danmaku {
            mode: u32,
            size: u32,
            color: u32,
            dmid: i32,
            text: String,
            r#type: u32,
            uid: u32,
            uname: String,
        },
        SendGift {
            action: String,
            gift_name: String,
            num: u32,
            uid: u32,
            uname: String,
        },
        ComboEnd {
            action: String,
            gift_name: String,
            num: u32,
            uid: u32,
            uname: String,
        },
        Welcome {
            /// 房管
            is_admin: bool,
            /// 年费 / 月费老爷
            is_svip: bool,
            uid: u32,
            uname: String,
        },
        WelcomeGuard {
            /// 舰队等级
            guard_level: GuardLevel,
            uid: u32,
            uname: String,
        },
        RoomRealTimeMessageUpdate {
            /// 粉丝数
            fans: u32,
        },
        HotRoomNotify,
        Raw(json::JsonValue),
        ParsingError(String),
    }

    impl Message {
        /// Parses a `ChatPacket` into a `Message`.
        ///
        /// If any field of the json is null,
        /// an `Err` containing string slice of the payload is returned.
        pub fn parse(mut json: json::JsonValue) -> Option<Message> {
            Some(match json["cmd"].as_str()? {
                "PREPARING" => Preparing,
                "LIVE" => Live,
                "ROOM_CHANGE" => {
                    let data = &mut json["data"];
                    RoomChange {
                        title: data["title"].take_string()?,
                        area_name: data["area_name"].take_string()?,
                        parent_area_name: data["parent_area_name"].take_string()?,
                    }
                }
                "DANMU_MSG" => {
                    let info = &mut json["info"];
                    Danmaku {
                        mode: info[0][1].as_u32()?,
                        size: info[0][2].as_u32()?,
                        color: info[0][3].as_u32()?,
                        dmid: info[0][5].as_i32()?,
                        text: info[1].take_string()?,
                        r#type: info[0][9].as_u32()?,
                        uid: info[2][0].as_u32()?,
                        uname: info[2][1].take_string()?,
                    }
                }
                "SEND_GIFT" => {
                    let data = &mut json["data"];
                    SendGift {
                        action: data["action"].take_string()?,
                        gift_name: data["giftName"].take_string()?,
                        num: data["num"].as_u32()?,
                        uid: data["uid"].as_u32()?,
                        uname: data["uname"].take_string()?,
                    }
                }
                "COMBO_END" => {
                    let data = &mut json["data"];
                    ComboEnd {
                        action: data["action"].take_string()?,
                        gift_name: data["gift_name"].take_string()?,
                        num: data["combo_num"].as_u32()?,
                        uid: data["uid"].as_u32()?,
                        uname: data["uname"].take_string()?,
                    }
                }
                "WELCOME" => {
                    let data = &mut json["data"];
                    assert!(data["vip"] == 1);
                    Welcome {
                        is_admin: data["is_admin"].as_bool()?,
                        is_svip: data["svip"] != 0,
                        uid: data["uid"].as_u32()?,
                        uname: data["uname"].take_string()?,
                    }
                }
                "WELCOME_GUARD" => {
                    let data = &mut json["data"];
                    let guard_level = data["guard_level"].as_u32()?;
                    WelcomeGuard {
                        guard_level: GuardLevel::from(guard_level)?,
                        uid: data["uid"].as_u32()?,
                        uname: data["username"].take_string()?,
                    }
                }
                "ROOM_REAL_TIME_MESSAGE_UPDATE" => {
                    let data = &mut json["data"];
                    RoomRealTimeMessageUpdate {
                        fans: data["fans"].as_u32()?,
                    }
                }
                "HOT_ROOM_NOTIFY" => HotRoomNotify,
                _ => Raw(json),
            })
        }
    }

    pub enum GuardLevel {
        /// 非舰队成员
        None,
        /// 舰长
        Captain,
        /// 提督
        Praefect,
        /// 总督
        Governor,
    }

    impl GuardLevel {
        fn from(n: u32) -> Option<Self> {
            Some(match n {
                0 => GuardLevel::None,
                1 => GuardLevel::Governor,
                2 => GuardLevel::Praefect,
                3 => GuardLevel::Captain,
                _ => return None,
            })
        }
    }

    impl fmt::Display for GuardLevel {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(match self {
                GuardLevel::None => "非舰队成员",
                GuardLevel::Captain => "舰长",
                GuardLevel::Praefect => "提督",
                GuardLevel::Governor => "总督",
            })
        }
    }
}
