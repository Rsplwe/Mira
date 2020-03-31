mod http_api {
    const API_ROOM_INIT: &str = "http://api.live.bilibili.com/room/v1/Room/room_init?id=";

    pub async fn get_room_id(id: u32) -> Result<u32, Box<dyn std::error::Error>> {
        let client = hyper::Client::new();
        let uri = format!("{}{}", API_ROOM_INIT, id).parse().unwrap();
        let resp = client.get(uri).await.unwrap();
        let bytes = hyper::body::to_bytes(resp).await?;
        let json: serde_json::Value = serde_json::from_slice(&bytes)?;
        Ok(json["data"]["room_id"].as_u64().unwrap() as u32)
    }
}

pub mod chat {
    use bytes::{Buf, BufMut, BytesMut};
    use futures_util::{
        sink::SinkExt,
        stream::{StreamExt, TryStreamExt},
    };
    use std::fmt::{self, Write};
    use std::io::Cursor;
    use tokio::io;
    use tokio::net::TcpStream;
    use tokio::time::{self, Duration};
    use tokio_util::codec::{Decoder, Encoder, Framed};

    const HOST: &str = "broadcastlv.chat.bilibili.com";
    const PORT: u16 = 2243;

    const HEARTBEAT_DELAY: Duration = Duration::from_secs(30);

    const HEADER_LENGTH: usize = 16;

    pub const PROTO_RAW_JSON: u16 = 0;
    pub const PROTO_HEARTBEAT: u16 = 1;
    pub const PROTO_COMPRESSED_JSON: u16 = 2;

    pub const TYPE_HEARTBEAT: u32 = 2;
    pub const TYPE_HEARTBEAT_RESPONSE: u32 = 3;
    pub const TYPE_ANNOUNCEMENT: u32 = 5;
    pub const TYPE_AUTHENTICATION: u32 = 7;
    pub const TYPE_AUTHENTICATION_RESPONSE: u32 = 8;

    pub async fn connect(
        id: u32,
    ) -> Result<impl TryStreamExt<Ok = ChatPacket, Error = Error>, Box<dyn std::error::Error>> {
        let id = super::http_api::get_room_id(id).await?;
        let addr = (HOST, PORT);
        let stream = TcpStream::connect(addr).await?;

        let (mut sink, stream) = Framed::new(stream, ChatCodec).split();
        sink.send(ChatPacket::authenticate(id)).await?;
        tokio::spawn(async move {
            loop {
                if let Err(e) = sink.send(ChatPacket::heartbeat()).await {
                    eprintln!("Failed to send heartbeat: {:?}", e);
                }
                time::delay_for(HEARTBEAT_DELAY).await;
            }
        });
        Ok(stream)
    }

    #[derive(Debug)]
    pub struct ChatPacket {
        pub proto_ver: u16,
        pub pk_type: u32,
        pub payload: Vec<u8>,
    }

    impl ChatPacket {
        fn authenticate(room_id: u32) -> ChatPacket {
            let mut payload = String::new();
            write!(payload, r#"{{"uid":0,"roomid":{}}}"#, room_id).unwrap();
            ChatPacket {
                proto_ver: PROTO_HEARTBEAT,
                pk_type: TYPE_AUTHENTICATION,
                payload: payload.into_bytes(),
            }
        }

        fn heartbeat() -> ChatPacket {
            ChatPacket {
                proto_ver: PROTO_HEARTBEAT,
                pk_type: TYPE_HEARTBEAT,
                payload: Vec::new(),
            }
        }
    }

    /// Codec for chat packets
    ///
    /// packet length: u32
    /// header length: u16 (16)
    /// protocol version: u16
    /// packet type: u32
    /// unknown: u32 (1)
    /// data: [u8]
    struct ChatCodec;

    impl Encoder<ChatPacket> for ChatCodec {
        type Error = io::Error;

        fn encode(&mut self, pk: ChatPacket, dst: &mut BytesMut) -> Result<(), Self::Error> {
            dst.put_u32((HEADER_LENGTH + pk.payload.len()) as u32);
            dst.put_u16(HEADER_LENGTH as u16);
            dst.put_u16(pk.proto_ver);
            dst.put_u32(pk.pk_type);
            dst.put_u32(1);
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
                src.reserve(len - src_len);
                return Ok(None);
            }
            src.advance(4);
            if src.get_u16() != HEADER_LENGTH as u16 {
                return Err(Error::Codec("unexpected header length"));
            }
            let proto_ver = src.get_u16();
            let pk_type = src.get_u32();
            src.advance(4); // Unknown field

            let len = len - HEADER_LENGTH;
            let mut payload = Vec::with_capacity(len);
            unsafe {
                payload.set_len(len);
            }
            src.copy_to_slice(&mut payload);
            Ok(Some(ChatPacket {
                proto_ver,
                pk_type,
                payload,
            }))
        }
    }

    #[derive(Debug)]
    pub enum Error {
        /// Codec error with description
        Codec(&'static str),
        /// An IO error occured.
        Io(io::Error),
    }

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Error::Codec(s) => f.write_str(s),
                Error::Io(e) => write!(f, "{}", e),
            }
        }
    }

    impl From<io::Error> for Error {
        fn from(e: io::Error) -> Error {
            Error::Io(e)
        }
    }

    impl std::error::Error for Error {}
}
