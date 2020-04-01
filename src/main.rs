use futures_util::stream::TryStreamExt;
use mira::chat::*;
use mira::msg::Message::*;
use std::io::{self, Write};

#[tokio::main]
async fn main() {
    print!("直播间 ID: ");
    io::stdout().flush().unwrap();
    let mut id = String::new();
    io::stdin().read_line(&mut id).unwrap();
    let id = id.trim_end().parse().unwrap();

    let stream = connect(id).await.unwrap();
    stream
        .try_for_each(|pk| async {
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
                    HotRoomNotify => println!("[热门直播间]"),
                    Raw(json) => println!("{}", json.to_string()),
                },
            }
            Ok(())
        })
        .await
        .unwrap();
}
