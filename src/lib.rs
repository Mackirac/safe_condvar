use std::collections::LinkedList;
use std::time::Duration;
use std::sync::{ self,
    atomic::{ Ordering, AtomicBool },
    Mutex, MutexGuard,
    LockResult, WaitTimeoutResult,
};

struct Condition {
    sleep: AtomicBool,
    condvar: sync::Condvar,
}

impl Default for Condition {
    fn default() -> Self {
        Condition {
            sleep: AtomicBool::new(true),
            condvar: sync::Condvar::new(),
        }
    }
}

pub struct Condvar {
    locked: Mutex<bool>,
    condvar: sync::Condvar,
    queue: *mut LinkedList<Box<Condition>>,
}

unsafe impl Send for Condvar {}
unsafe impl Sync for Condvar {}

macro_rules! atomic {
    ($instance:expr => $($commands:tt)*) => {
        $instance.entry_protocol();
        $($commands)*
        $instance.exit_protocol();
    };
}

impl Condvar {
    pub fn new() -> Self {
        Condvar {
            locked: Mutex::new(false),
            condvar: sync::Condvar::new(),
            queue: Box::into_raw(Box::new(LinkedList::new())),
        }
    }

    fn entry_protocol(&self) {
        let mut locked = self.locked.lock().unwrap();
        locked = self.condvar
                     .wait_while(
                         locked,
                         |locked| *locked
                     )
                     .unwrap();
        *locked = true;
    }

    fn exit_protocol(&self) {
        *self.locked.lock().unwrap() = false;
    }

    fn wait_in_queue(&self) -> &Condition {
        atomic! { self =>
            let queue = unsafe { self.queue.as_mut().unwrap() };
            queue.push_back(Default::default());
            let condition: &Condition = queue.back().unwrap();
        }
        condition
    }

    pub fn empty(&self) -> bool {
        atomic! { self => let empty = unsafe { (*self.queue).is_empty() }; }
        empty
    }

    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> LockResult<MutexGuard<'a, T>> {
        let _condition = self.wait_in_queue();
        _condition.condvar.wait_while(
            guard,
            |_| { _condition.sleep.load(Ordering::Relaxed) }
        )
    }

    pub fn wait_timeout<'a, T> (
        &self,
        guard: MutexGuard<'a, T>,
        duration: Duration
    ) -> LockResult<(MutexGuard<'a, T>, WaitTimeoutResult)>
    {
        let _condition = self.wait_in_queue();
        _condition.condvar.wait_timeout_while(guard, duration,
            |_| { _condition.sleep.load(Ordering::Relaxed) }
        )
    }

    pub fn wait_while<'a, T, F>(
        &self,
        mut guard: MutexGuard<'a, T>,
        mut condition: F
    ) -> LockResult<MutexGuard<'a, T>> where
        F: FnMut(&mut T) -> bool
    {
        while condition(&mut guard) { guard = self.wait(guard)?; };
        Ok(guard)
    }

    pub fn wait_timeout_while<'a, T, F>(
        &self,
        mut guard: MutexGuard<'a, T>,
        duration: Duration,
        mut condition: F
    ) -> LockResult<(MutexGuard<'a, T>, WaitTimeoutResult)> where
        F: FnMut(&mut T) -> bool
    {
        let mut timeout = construct_empty_timeout();
        while condition(&mut guard) {
            let result = self.wait_timeout(guard, duration)?;
            guard = result.0;
            timeout = result.1;
        }
        Ok((guard, timeout))
    }

    pub fn notify_one(&self) {
        atomic! { self => let condition = unsafe { (*self.queue).pop_front() }; }
        if let Some(condition) = condition {
            condition.sleep.store(false, Ordering::Relaxed);
            condition.condvar.notify_one();
        }
    }

    pub fn notify_all(&self) {
        atomic! { self =>
            while let Some(condition) = unsafe { (*self.queue).pop_front() } {
                condition.sleep.store(false, Ordering::Relaxed);
                condition.condvar.notify_one();
            }
        }
    }
}

fn construct_empty_timeout() -> WaitTimeoutResult {
    Condvar::new().wait_timeout_while(
        Mutex::new(()).lock().unwrap(),
        std::time::Duration::from_secs(1),
        |_| false
    ).unwrap().1
}

#[cfg(test)]
mod tests {
    use super::Condvar;
    use std::sync::{ Arc, Mutex };
    #[test]
    fn main_test() {
        let data = Arc::new((Mutex::new(()), Condvar::new()));

        let data_clone = data.clone();
        let thread = std::thread::spawn(move || {
            let guard = data_clone.0.lock().unwrap();
            println!("Secondary thread waiting for notification...");
            println!("{:?}", data_clone.1.wait(guard).unwrap());
        });

        println!("Main thread going to sleep...");
        std::thread::sleep(std::time::Duration::from_secs(2));
        println!("Main thread sleep time is over");
        data.1.notify_one();

        thread.join().unwrap();
    }

    #[test]
    fn timeout() {
        use std::sync::{ Arc, Mutex };
        let data1 = Arc::new((Mutex::new(()), Condvar::new()));
        let data2 = data1.clone();

        let t = std::thread::spawn(move || {
            let lock = data2.0.lock().unwrap();
            println!("{:?}", data2.1.wait_timeout(lock, std::time::Duration::from_secs(1)));
        });

        std::thread::sleep(std::time::Duration::from_secs(2));

        t.join().unwrap();
    }
}
