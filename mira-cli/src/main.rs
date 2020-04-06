use mira_core::chat::*;
use mira_core::msg::Message::*;
use std::io::{self, Write};

#[tokio::main]
async fn main() {
    print!("直播间 ID: ");
    io::stdout().flush().unwrap();
    let mut id = String::new();
    io::stdin().read_line(&mut id).unwrap();
    let id = id.trim_end().parse().unwrap();
    connect(id, handle_packet).await.unwrap();

    // let (fut, handle) = futures_util::future::abortable(connect(id, handle_packet));
    // tokio::spawn(fut);
    // tokio::time::delay_for(tokio::time::Duration::from_secs(5)).await;
    // handle.abort();
}

async fn handle_packet(pk: ChatPacket) {
    match pk {
        ChatPacket::ConnectSuccess => {
            println!("成功连接到 Bilibili 弹幕服务器");
        }
        ChatPacket::Popularity(p) => {
            println!("[人气值] {}", p);
        }
        ChatPacket::Message(msg) => match msg {
            Live => println!("[开播]"),
            Preparing => println!("[下播]"),
            RoomChange {
                title,
                area_name,
                parent_area_name,
            } => {
                println!(
                    "[直播间信息更改] [{}·{}] {}",
                    parent_area_name, area_name, title
                );
            }
            Danmaku { uname, text, .. } => {
                println!("{}: {}", uname, text);
            }
            SendGift {
                uname,
                action,
                gift_name,
                num,
                ..
            }
            | ComboEnd {
                uname,
                action,
                gift_name,
                num,
                ..
            } => {
                println!("{}: {} {} * {}", uname, action, gift_name, num);
            }
            Welcome {
                uname,
                is_admin,
                is_svip,
                ..
            } => {
                let what = if is_admin {
                    "房管"
                } else if is_svip {
                    "年费老爷"
                } else {
                    "月费老爷"
                };
                println!("{} {} 进入直播间", uname, what)
            }
            WelcomeGuard {
                guard_level, uname, ..
            } => {
                eprintln!("欢迎 {} {} 进入直播间", guard_level, uname);
            }
            RoomRealTimeMessageUpdate { fans } => {
                println!("[粉丝数] {}", fans);
            }
            RoomRank { rank_desc, color, timestamp, } => {
                println!("[房间排行榜/{}][{}] {}", color, timestamp, rank_desc);
            }
            NoticeMessage {
                roomid,
                real_roomid,
                msg_common,
                msg_self,
            } => {
                println!("[通知消息] {}", msg_common)
            }
            SuperChatMessage {
                id,
                sender_uid,
                price,
                message,
                sender_name
            } => {
                println!("[SC] {} 置顶了消息 {} ({}元)", sender_name, message, price)
            }
            SuperChatMessageJapanese {
                id,
                sender_uid,
                price,
                message,
                message_jpn,
                sender_name
            } => {
                println!("[SC] {} がメッセージをピン留めしました： {} ({}RMB)", sender_name, message_jpn, price)
            }
            HotRoomNotify => println!("[热门直播间]"),
            Raw(json) => println!("{}", json.to_string()),
            ParsingError(str) => panic!("failed to parse json: {}", str),
            _ => {}
        },
    }
}
