use bytes::Bytes;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{self, AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc::Sender;

use crate::domain::progress::RsProgress;

const PROGRESS_UPDATE_BYTES: usize = 4 * 1024 * 1024;

pub struct ProgressReader<R> {
    pub inner: R,
    pub bytes_read: usize,
    pub bytes_reported: usize,
    pub progress_template: RsProgress,
    pub sender: Sender<RsProgress>,
}

impl<R> Drop for ProgressReader<R> {
    fn drop(&mut self) {
        let sender = self.sender.clone();
        let mut new_progress = self.progress_template.clone();
        new_progress.current = Some(self.bytes_read as u64);
        new_progress.total = Some(self.bytes_read as u64);

        tokio::spawn(async move {
            let _ = sender.send(new_progress).await;
        });
    }
}

impl<R> ProgressReader<R> {
    pub fn new(inner: R, progress_template: RsProgress, sender: Sender<RsProgress>) -> Self {
        Self {
            inner,
            progress_template,
            sender,
            bytes_read: 0,
            bytes_reported: 0,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for ProgressReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = poll {
            let read = buf.filled().len().saturating_sub(before);
            self.bytes_read += read;
            let mut new_progress = self.progress_template.clone();
            new_progress.current = Some(self.bytes_read as u64);
            if read > 0
                && self.bytes_read.saturating_sub(self.bytes_reported) >= PROGRESS_UPDATE_BYTES
            {
                self.bytes_reported = self.bytes_read;
                let _ = self.sender.try_send(new_progress);
            }
        }
        poll
    }
}
