//! An AI job's queue (`architecture.md#jobs`): the items it has a model
//! take one after the other on one account (the email analysis, the reading
//! of attachments and of scans), what a stop left, whether the spend limit
//! stopped it, and its failures, the first one in words.

use std::collections::VecDeque;

use av_utils::datetime::DateTime;
use av_utils::id::Id;

use crate::ai::accounts::AiAccounts;
use crate::texts::Text;

// ----------------------------------------------------------------------------
pub struct AiQueue<T> {
    /// The model's account; none where nothing needs a model (a PDF watch
    /// that reads text layers only).
    account: Option<Id>,
    waiting: VecDeque<T>,
    /// The item the model has; stopping the job cancels it.
    current: Option<T>,
    total: usize,
    /// Not done: stopped, or the spend limit reached.
    left: usize,
    /// Stopped at the account's spend limit.
    limit: bool,
    failed: usize,
    /// The first failure, in words, for the notice.
    first_error: Option<Text>,
}

impl<T> AiQueue<T> {
    // ------------------------------------------------------------------------
    pub fn new(account: Option<Id>, items: VecDeque<T>) -> Self {
        Self {
            account,
            total: items.len(),
            waiting: items,
            current: None,
            left: 0,
            limit: false,
            failed: 0,
            first_error: None,
        }
    }

    // ------------------------------------------------------------------------
    /// `items` after those waiting.
    pub fn extend(&mut self, items: impl IntoIterator<Item = T>) {
        for item in items {
            self.waiting.push_back(item);
            self.total += 1;
        }
    }

    // ------------------------------------------------------------------------
    pub fn account(&self) -> Option<Id> {
        self.account
    }

    // ------------------------------------------------------------------------
    pub fn is_busy(&self) -> bool {
        self.current.is_some()
    }

    // ------------------------------------------------------------------------
    /// Whether no item waits.
    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }

    // ------------------------------------------------------------------------
    pub fn current(&self) -> Option<&T> {
        self.current.as_ref()
    }

    // ------------------------------------------------------------------------
    pub fn waiting(&self) -> impl Iterator<Item = &T> {
        self.waiting.iter()
    }

    // ------------------------------------------------------------------------
    pub fn front(&self) -> Option<&T> {
        self.waiting.front()
    }

    // ------------------------------------------------------------------------
    /// The next item, done without the model (an attachment with text
    /// already): it counts as done.
    pub fn skip(&mut self) -> Option<T> {
        self.waiting.pop_front()
    }

    // ------------------------------------------------------------------------
    /// Whether the account may take another item now: the items so far are
    /// counted (`ai::account_used`). At its spend limit the queue is stopped
    /// by it, and the refusal's words come back.
    pub fn check_limit(&mut self, ai: &AiAccounts) -> Result<(), Text> {
        let Some(account) = self.account else {
            return Ok(());
        };
        ai.check_limit(account, DateTime::now()).map_err(|refusal| {
            self.limit = true;
            refusal.text
        })
    }

    // ------------------------------------------------------------------------
    /// The next item, now the model's.
    pub fn start(&mut self) -> Option<&T> {
        let item = self.waiting.pop_front()?;
        Some(self.current.insert(item))
    }

    // ------------------------------------------------------------------------
    /// The model answered for its item: done, unless it is asked again.
    pub fn end(&mut self) -> Option<T> {
        self.current.take()
    }

    // ------------------------------------------------------------------------
    /// `item` asked about again, first.
    pub fn again(&mut self, item: T) {
        self.waiting.push_front(item);
    }

    // ------------------------------------------------------------------------
    /// An item failed with `error`; the first failure is kept in words, with
    /// the item's `title`.
    pub fn fail(&mut self, title: &str, error: Text) {
        self.failed += 1;
        self.first_error.get_or_insert_with(|| {
            Text::new("ai.item_failed")
                .arg("title", title)
                .arg("error", error)
        });
    }

    // ------------------------------------------------------------------------
    /// Stop: the model's item and those waiting are left, and come back,
    /// the model's first.
    pub fn stop(&mut self) -> Vec<T> {
        let mut left: Vec<T> = self.current.take().into_iter().collect();
        left.extend(self.waiting.drain(..));
        self.left += left.len();
        left
    }

    // ------------------------------------------------------------------------
    /// The items done: neither waiting, the model's, nor left.
    pub fn done(&self) -> usize {
        self.total - self.waiting.len() - usize::from(self.current.is_some()) - self.left
    }

    // ------------------------------------------------------------------------
    pub fn total(&self) -> usize {
        self.total
    }

    // ------------------------------------------------------------------------
    pub fn failed(&self) -> usize {
        self.failed
    }

    // ------------------------------------------------------------------------
    pub fn left(&self) -> usize {
        self.left
    }

    // ------------------------------------------------------------------------
    pub fn limit(&self) -> bool {
        self.limit
    }

    // ------------------------------------------------------------------------
    /// The failures in one notice, "`job`: 2 of 5 files couldn't be read;
    /// “a.pdf”: …", `failed` saying how many of how many (`failed`, `n`);
    /// none without a failure.
    pub fn failures(&self, job: Text, failed: Text) -> Option<Text> {
        let error = self.first_error.clone()?;
        let failed = failed.arg("failed", self.failed).arg("n", self.total);
        let text = Text::new("ai.failures")
            .arg("failed", failed)
            .arg("error", error);
        Some(Text::new("job.notice").arg("job", job).arg("text", text))
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    fn queue(items: &[&'static str]) -> AiQueue<&'static str> {
        AiQueue::new(None, items.iter().copied().collect())
    }

    // ------------------------------------------------------------------------
    #[test]
    fn done_counts_what_is_neither_waiting_the_models_nor_left() {
        let mut q = queue(&["a", "b", "c", "d"]);
        assert_eq!((q.done(), q.total()), (0, 4));
        assert_eq!(q.skip(), Some("a"));
        assert_eq!(q.start(), Some(&"b"));
        assert!(q.is_busy());
        assert_eq!(q.done(), 1, "the model's isn't done");
        let b = q.end().unwrap();
        q.again(b);
        assert_eq!(q.done(), 1, "asked again");
        assert_eq!(q.front(), Some(&"b"));
        q.start();
        q.end();
        assert_eq!(q.done(), 2);
        q.extend(["e"]);
        assert_eq!((q.done(), q.total()), (2, 5));
        q.start();
        assert_eq!(q.stop(), ["c", "d", "e"], "the model's first");
        assert_eq!((q.done(), q.left()), (2, 3));
        assert!(q.is_empty() && !q.is_busy());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_failures_notice_names_the_first() {
        let mut q = queue(&["a", "b"]);
        let job = || Text::new("attachments.reading").arg("email", "jan@example.com");
        let failed = || Text::new("ai.failed.files");
        assert_eq!(q.failures(job(), failed()), None);
        q.fail("a.pdf", Text::new("reader.no_text"));
        q.fail("b.pdf", Text::new("reader.unread"));
        assert_eq!(q.failed(), 2);
        assert_eq!(
            q.failures(job(), failed()).unwrap().render(),
            "Reading attachments \u{b7} jan@example.com: 2 of 2 files couldn't be read; \u{201c}a.pdf\u{201d}: The file has no text."
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn without_an_account_there_is_no_limit() {
        let mut q = queue(&["a"]);
        assert_eq!(q.check_limit(&AiAccounts::default()), Ok(()));
        assert!(!q.limit());
    }
}
