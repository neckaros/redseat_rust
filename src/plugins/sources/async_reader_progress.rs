use tokio::io::{self, AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc::Sender;
use std::pin::Pin;
use std::task::{Context, Poll};
use bytes::Bytes;

use crate::domain::progress::RsProgress;
pub struct ProgressReader<R> {
    pub inner: R,
    pub bytes_read: usize,
    pub progress_template: RsProgress,
    pub sender: Sender<RsProgress>
}


impl<R> Drop for ProgressReader<R> {
    fn drop(&mut self) {
        let sender = self.sender.clone();
        let mut new_progress = self.progress_template.clone();
        new_progress.current = Some(self.bytes_read as u64); 
        new_progress.total = Some(self.bytes_read as u64);

        tokio::spawn(async move {
            sender.send(new_progress).await.unwrap();
        });
    }
}

impl<R> ProgressReader<R> {
    pub fn new(inner: R, progress_template: RsProgress, sender: Sender<RsProgress>) -> Self {
        Self {
            inner,
            progress_template,
            sender,
            bytes_read: 0
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for ProgressReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = poll {
            self.bytes_read += buf.filled().len();
            let mut new_progress = self.progress_template.clone();
            new_progress.current = Some(self.bytes_read as u64); 
            let sender = self.sender.clone();
            tokio::spawn(async move {
                sender.send(new_progress).await.unwrap();
            });
            
        }
        poll
    }
}