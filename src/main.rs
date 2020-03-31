use bytes::Buf;
use futures_util::future;
use futures_util::stream::TryStreamExt;
use mira::chat::*;

#[tokio::main]
async fn main() {
    let stream = connect(14047).await.unwrap();
    println!("Connected to Bilibili Chat Server");
    stream
        .try_for_each(|pk| {
            match pk.pk_type {
                TYPE_HEARTBEAT_RESPONSE => {
                    let mut payload = &pk.payload[..];
                    println!("Popularity: {}", payload.get_u32());
                }
                TYPE_ANNOUNCEMENT => {
                    let json: serde_json::Value = serde_json::from_slice(&pk.payload[..]).unwrap();
                    if json["cmd"] == "DANMU_MSG" {
                        println!(
                            "{}: {}",
                            json["info"][2][1].as_str().unwrap(),
                            json["info"][1].as_str().unwrap(),
                        );
                    } else {
                        println!("{}", json);
                    }
                }
                _ => (),
            }
            future::ready(Ok(()))
        })
        .await
        .unwrap();
}
