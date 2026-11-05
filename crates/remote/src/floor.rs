//! 发言权：同一时间只有一个声源在说，候选只归最后说话的那一个。
//!
//! 声源不只是手机——电脑本机麦克风也要来抢同一份发言权，所以它不能长在 [`crate::Service`] 里：
//! 服务会因为隧道断开整套重启，发言权和事件通道要活得比它久。由上层建一次，交给每一代服务。
//!
//! 它只是一把锁加一条通道，不碰网络：本机麦克风用它时不需要隧道、WebRTC 或任何手机在线，
//! 也不会被算成一台手机。
//!
//! 所有声源的事件走同一条通道，这是顺序保证的来源：结束一句的一方必须**先发出结束事件、
//! 再交还发言权**（见 [`Floor::give_back`]），下一位拿到发言权后发的 Press 一定排在它后面。

use crate::{Conn, Event};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// 电脑本机麦克风的编号。手机连接从 1 开始递增，碰不到这个值；0 留给「没有人」
pub const LOCAL: Conn = u64::MAX;

pub struct Floor {
    /// 正在说话的声源，0 表示没人。触摸板回调是同步的，所以用原子量而不是锁
    holding: AtomicU64,
    /// 最后说话的声源：松手后候选还在，只有它能选
    owner: AtomicU64,
    events: mpsc::UnboundedSender<Event>,
}

impl Floor {
    /// 发言权与它的事件通道。接收端只能有一个读者，事件顺序就是会话看到的顺序
    pub fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<Event>) {
        let (events, rx) = mpsc::unbounded_channel();
        (Arc::new(Self { holding: AtomicU64::new(0), owner: AtomicU64::new(0), events }), rx)
    }

    pub fn emit(&self, e: Event) {
        let _ = self.events.send(e);
    }

    /// 抢发言权。成功后它就是候选的主人
    pub fn take(&self, who: Conn) -> bool {
        let ok = self.holding.compare_exchange(0, who, Ordering::AcqRel, Ordering::Acquire).is_ok();
        if ok {
            self.owner.store(who, Ordering::Release);
        }
        ok
    }

    /// 交还发言权。调用前必须已经发出这一句的结束事件（Release / Cancel / Abort），
    /// 否则下一位的 Press 可能抢在它前面进会话。不是持有者就什么都不做
    pub fn give_back(&self, who: Conn) -> bool {
        self.holding.compare_exchange(who, 0, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    /// 正在说话的声源，0 表示没人
    pub fn speaker(&self) -> Conn {
        self.holding.load(Ordering::Acquire)
    }

    /// 没人在说，或者调用者就是说话的人：可以动鼠标、退格
    pub fn may_act(&self, who: Conn) -> bool {
        let holding = self.speaker();
        holding == 0 || holding == who
    }

    /// 候选归最后说话的声源；它走了就谁都能选
    pub fn may_commit(&self, who: Conn) -> bool {
        let owner = self.owner.load(Ordering::Acquire);
        owner == 0 || owner == who
    }

    /// 声源离开（手机断开、本机麦克风关闭）：不再占着候选
    pub fn leave(&self, who: Conn) {
        let _ = self.owner.compare_exchange(who, 0, Ordering::AcqRel, Ordering::Acquire);
    }

    /// 手机接入整套关闭：手机拿着的发言权和候选一并收回。
    ///
    /// 正常情况下每条连接退出时会自己交还，这里是兜底——发言权活得比服务久，
    /// 万一哪条连接的任务没走完收尾就没了，电脑本机会被永远挡在外面。
    /// 晚到的收尾不会出错：交还认的是持有者，会话认的是说话方，两边都对不上就作废
    pub(crate) fn drop_phones(&self) {
        for slot in [&self.holding, &self.owner] {
            let _ = slot.fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| (v != LOCAL && v != 0).then_some(0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_speaker_at_a_time() {
        let (f, _rx) = Floor::new();
        assert!(f.take(1));
        assert!(!f.take(LOCAL), "手机在说时本机抢不到");
        assert!(!f.give_back(LOCAL), "不是持有者，交还不算数");
        assert_eq!(f.speaker(), 1);
        assert!(f.give_back(1));
        assert!(f.take(LOCAL));
        assert_eq!(f.speaker(), LOCAL);
    }

    #[test]
    fn acting_and_committing_follow_the_speaker() {
        let (f, _rx) = Floor::new();
        assert!(f.may_act(2) && f.may_commit(2), "没人说话时谁都行");
        f.take(1);
        assert!(f.may_act(1) && !f.may_act(2));
        f.give_back(1);
        assert!(f.may_act(2), "松手后鼠标谁都能动");
        assert!(f.may_commit(1) && !f.may_commit(2), "候选还归刚说完的那个");
        f.leave(2);
        assert!(!f.may_commit(2), "别人离开不影响候选归属");
        f.leave(1);
        assert!(f.may_commit(2));
    }

    /// 服务关闭只收回手机的，本机正在说的那句不受影响
    #[test]
    fn dropping_phones_keeps_local() {
        let (f, _rx) = Floor::new();
        f.take(3);
        f.drop_phones();
        assert_eq!(f.speaker(), 0);
        assert!(f.may_commit(1), "断掉的手机不再占着候选");
        assert!(!f.give_back(3), "晚到的交还不会误放别人的发言权");

        f.take(LOCAL);
        f.drop_phones();
        assert_eq!(f.speaker(), LOCAL);
        assert!(!f.may_commit(1));
    }

    /// 先发结束事件再交还：下一位的 Press 一定排在后面
    #[test]
    fn ending_event_precedes_next_press() {
        let (f, mut rx) = Floor::new();
        let (_tx, audio) = mpsc::unbounded_channel();
        f.take(1);
        f.emit(Event::Release(1));
        f.give_back(1);
        assert!(f.take(LOCAL));
        f.emit(Event::Press(LOCAL, audio));
        assert!(matches!(rx.try_recv(), Ok(Event::Release(1))));
        assert!(matches!(rx.try_recv(), Ok(Event::Press(LOCAL, _))));
    }
}
