use crate::tls::{build_client_config, build_server_config, NodeIdentity, TrustPolicy};
use crate::NetError;
use mouseroaming_proto::{decode_frame, encode_frame, Message, MAX_FRAME_SIZE};
use rustls_pki_types::ServerName;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};

pub struct MessageConnection {
    stream: TlsStream<TcpStream>,
}

impl MessageConnection {
    pub fn split(self) -> (MessageReader, MessageWriter) {
        let (r, w) = tokio::io::split(self.stream);
        (MessageReader { reader: r }, MessageWriter { writer: w })
    }

    pub async fn send(&mut self, msg: &Message) -> Result<(), NetError> {
        let bytes = encode_frame(msg)?;
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Message, NetError> {
        read_framed(&mut self.stream).await
    }
}

pub struct MessageReader {
    reader: ReadHalf<TlsStream<TcpStream>>,
}

impl MessageReader {
    pub async fn recv(&mut self) -> Result<Message, NetError> {
        read_framed(&mut self.reader).await
    }
}

pub struct MessageWriter {
    writer: WriteHalf<TlsStream<TcpStream>>,
}

impl MessageWriter {
    pub async fn send(&mut self, msg: &Message) -> Result<(), NetError> {
        let bytes = encode_frame(msg)?;
        self.writer.write_all(&bytes).await?;
        self.writer.flush().await?;
        Ok(())
    }
}

async fn read_framed<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Message, NetError> {
    let mut header = [0u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(NetError::Closed);
        }
        Err(e) => return Err(NetError::Io(e)),
    }
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME_SIZE {
        return Err(NetError::Proto(
            mouseroaming_proto::ProtoError::FrameTooLarge(len, MAX_FRAME_SIZE),
        ));
    }
    let mut buf = vec![0u8; 4 + len];
    buf[..4].copy_from_slice(&header);
    reader.read_exact(&mut buf[4..]).await?;
    Ok(decode_frame(&buf)?)
}

pub struct TlsServerListener {
    listener: TcpListener,
    acceptor: TlsAcceptor,
}

impl TlsServerListener {
    pub async fn bind(addr: SocketAddr, identity: &NodeIdentity) -> Result<Self, NetError> {
        let server_cfg = build_server_config(identity)?;
        let listener = TcpListener::bind(addr).await?;
        Ok(Self {
            listener,
            acceptor: TlsAcceptor::from(server_cfg),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, NetError> {
        Ok(self.listener.local_addr()?)
    }

    pub async fn accept(&self) -> Result<(MessageConnection, SocketAddr), NetError> {
        let (tcp, peer_addr) = self.listener.accept().await?;
        let _ = tcp.set_nodelay(true);
        let tls = self.acceptor.accept(tcp).await?;
        Ok((
            MessageConnection {
                stream: TlsStream::Server(tls),
            },
            peer_addr,
        ))
    }
}

pub async fn connect_tls(
    addr: &str,
    policy: TrustPolicy,
) -> Result<(MessageConnection, String), NetError> {
    let observed = Arc::new(Mutex::new(None));
    let client_cfg = build_client_config(policy, observed.clone())?;
    let connector = TlsConnector::from(client_cfg);

    let tcp = TcpStream::connect(addr).await?;
    let _ = tcp.set_nodelay(true);

    let server_name = ServerName::try_from("mouseroaming.local".to_string())
        .map_err(|e| NetError::InvalidAddress(e.to_string()))?;

    let tls = connector
        .connect(server_name, tcp)
        .await
        .map_err(|e| NetError::UntrustedPeer(e.to_string()))?;

    let fp = observed
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default();

    Ok((
        MessageConnection {
            stream: TlsStream::Client(tls),
        },
        fp,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::generate_ephemeral_identity;
    use mouseroaming_proto::Edge;

    #[tokio::test]
    async fn tls_round_trip_and_mitm_rejection() {
        let server_id = generate_ephemeral_identity("server-test").expect("id");
        let listener = TlsServerListener::bind("127.0.0.1:0".parse().unwrap(), &server_id)
            .await
            .expect("bind");
        let addr = listener.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.expect("accept");
            let msg = conn.recv().await.expect("recv");
            conn.send(&msg).await.expect("echo");
        });

        // 1. Valid pinned fingerprint succeeds
        let (mut client, fp) = connect_tls(
            &addr.to_string(),
            TrustPolicy::TrustOnFirstUse {
                expected_fingerprint: Some(server_id.fingerprint.clone()),
            },
        )
        .await
        .expect("connect");

        assert_eq!(fp, server_id.fingerprint);
        let sample = Message::Enter {
            edge: Edge::Right,
            ratio: 0.25,
        };
        client.send(&sample).await.expect("send");
        let echoed = client.recv().await.expect("recv");
        assert_eq!(echoed, sample);
        server_task.await.unwrap();

        // 2. Wrong pinned fingerprint must be rejected
        let listener2 = TlsServerListener::bind("127.0.0.1:0".parse().unwrap(), &server_id)
            .await
            .expect("bind2");
        let addr2 = listener2.local_addr().unwrap();
        let _server_task2 = tokio::spawn(async move {
            let _ = listener2.accept().await;
        });

        let bad = connect_tls(
            &addr2.to_string(),
            TrustPolicy::TrustOnFirstUse {
                expected_fingerprint: Some("DEADBEEF".into()),
            },
        )
        .await;
        assert!(bad.is_err(), "mismatched fingerprint must be rejected");
    }
}
