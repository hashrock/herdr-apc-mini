//! herdr の socket API クライアント。
//!
//! `$HERDR_SOCKET_PATH` の Unix domain socket に JSON Lines で話す。
//!
//! サーバは **1 接続につき 1 リクエスト**で接続を閉じる（CLI が 1 コマンド 1 接続
//! なのと同じ）。購読だけは接続が開いたままイベントが流れ続けるので、
//! 問い合わせ用の使い捨て接続と、購読用の長命な接続を分ける。

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    pub fn connect() -> Result<Self, Box<dyn std::error::Error>> {
        let path = std::env::var("HERDR_SOCKET_PATH")
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_default();
                format!("{home}/.config/herdr/herdr.sock")
            });
        let stream = UnixStream::connect(&path)
            .map_err(|e| format!("herdr socket {path} に接続できません: {e}"))?;
        Ok(Client {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
        })
    }

    fn send(&mut self, req: Value) -> Result<(), Box<dyn std::error::Error>> {
        self.writer.write_all(req.to_string().as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    /// 次の 1 行を読む。応答もイベントも区別せずに返す。
    pub fn next(&mut self) -> Result<Option<Value>, Box<dyn std::error::Error>> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if line.trim().is_empty() {
            return Ok(Some(Value::Null));
        }
        Ok(Some(serde_json::from_str(&line)?))
    }

    /// 購読を開始する。以降この接続にはイベントだけが流れてくる。
    pub fn subscribe(&mut self, kinds: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
        let subs: Vec<Value> = kinds.iter().map(|k| json!({"type": k})).collect();
        self.send(json!({
            "id": "sub",
            "method": "events.subscribe",
            "params": {"subscriptions": subs}
        }))?;
        let Some(ack) = self.next()? else {
            return Err("購読の応答が来ませんでした".into());
        };
        if let Some(err) = ack.get("error") {
            return Err(format!("購読に失敗しました: {err}").into());
        }
        Ok(())
    }
}

/// 使い捨ての接続で 1 リクエストだけ投げる。
pub fn request(method: &str, params: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let mut c = Client::connect()?;
    c.send(json!({"id": "req", "method": method, "params": params}))?;
    let Some(msg) = c.next()? else {
        return Err(format!("{method} の応答が来ませんでした").into());
    };
    if let Some(err) = msg.get("error") {
        return Err(format!("{method} が失敗しました: {err}").into());
    }
    Ok(msg["result"].clone())
}

/// エージェントの状態。herdr の AgentStatus と対応する。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Blocked,
    Working,
    Done,
    Idle,
    Unknown,
}

impl Status {
    pub fn parse(s: &str) -> Status {
        match s {
            "blocked" => Status::Blocked,
            "working" => Status::Working,
            "done" => Status::Done,
            "idle" => Status::Idle,
            _ => Status::Unknown,
        }
    }

    /// ワークスペース内に複数エージェントがいるときの優先順位。
    /// 手が要るものほど強い。
    fn rank(self) -> u8 {
        match self {
            Status::Blocked => 4,
            Status::Done => 3,
            Status::Working => 2,
            Status::Idle => 1,
            Status::Unknown => 0,
        }
    }

    pub fn max(self, other: Status) -> Status {
        if other.rank() > self.rank() { other } else { self }
    }
}
