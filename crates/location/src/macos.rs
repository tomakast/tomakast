//! Core Location. A `CLLocationManager` calls its delegate on the run loop of the thread that
//! made it, so the calling thread runs its own run loop until an answer or the timeout. The
//! system grants location only to an app bundle with `NSLocationUsageDescription`; a launcher
//! started outside one may be refused, or never asked.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_core_location::{
    CLAuthorizationStatus, CLLocation, CLLocationManager, CLLocationManagerDelegate,
    kCLLocationAccuracyThreeKilometers,
};
use objc2_foundation::{NSArray, NSDate, NSDefaultRunLoopMode, NSError, NSRunLoop};

use crate::Fix;

type Answer = Result<Fix, String>;

#[derive(Default)]
struct Ivars {
    answer: RefCell<Option<Answer>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Delegate` does not implement
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "TomakastLocationDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl CLLocationManagerDelegate for Delegate {
        #[unsafe(method(locationManager:didUpdateLocations:))]
        fn did_update_locations(
            &self,
            _manager: &CLLocationManager,
            locations: &NSArray<CLLocation>,
        ) {
            if let Some(last) = locations.lastObject() {
                // SAFETY: a location the manager handed over.
                let (at, accuracy) = unsafe { (last.coordinate(), last.horizontalAccuracy()) };
                self.answer(Ok(Fix {
                    latitude: at.latitude,
                    longitude: at.longitude,
                    // Negative: not valid.
                    accuracy_m: Some(accuracy).filter(|m| *m > 0.),
                }));
            }
        }

        #[unsafe(method(locationManager:didFailWithError:))]
        fn did_fail(&self, _manager: &CLLocationManager, error: &NSError) {
            self.answer(Err(format!(
                "Core Location failed: {}",
                error.localizedDescription()
            )));
        }

        #[unsafe(method(locationManagerDidChangeAuthorization:))]
        fn did_change_authorization(&self, manager: &CLLocationManager) {
            // SAFETY: called on the manager's own thread.
            let status = unsafe { manager.authorizationStatus() };
            if status == CLAuthorizationStatus::Denied
                || status == CLAuthorizationStatus::Restricted
            {
                self.answer(Err("location was not allowed for the launcher".into()));
            } else if status != CLAuthorizationStatus::NotDetermined {
                // SAFETY: as above.
                unsafe { manager.requestLocation() };
            }
        }
    }
);

impl Delegate {
    fn new() -> Retained<Delegate> {
        let this = Delegate::alloc().set_ivars(Ivars::default());
        // SAFETY: NSObject's `init`.
        unsafe { msg_send![super(this), init] }
    }

    /// Keeps the first answer.
    fn answer(&self, answer: Answer) {
        let mut slot = self.ivars().answer.borrow_mut();
        if slot.is_none() {
            *slot = Some(answer);
        }
    }

    fn take(&self) -> Option<Answer> {
        self.ivars().answer.borrow_mut().take()
    }
}

pub(crate) fn locate(timeout: Duration) -> Answer {
    // SAFETY: a class method without arguments.
    if !unsafe { CLLocationManager::locationServicesEnabled_class() } {
        return Err("location services are off".into());
    }
    let delegate = Delegate::new();
    // SAFETY: the manager is made, used and dropped on this thread, which runs its run loop
    // below; the delegate outlives it (the manager holds it weakly).
    let manager = unsafe { CLLocationManager::new() };
    unsafe {
        manager.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        manager.setDesiredAccuracy(kCLLocationAccuracyThreeKilometers);
        if manager.authorizationStatus() == CLAuthorizationStatus::NotDetermined {
            // The answer comes to `did_change_authorization`, which then asks.
            manager.requestWhenInUseAuthorization();
        } else {
            manager.requestLocation();
        }
    }
    let run_loop = NSRunLoop::currentRunLoop();
    let deadline = Instant::now() + timeout;
    let answer = loop {
        if let Some(answer) = delegate.take() {
            break answer;
        }
        if Instant::now() >= deadline {
            break Err(format!("no location within {} s", timeout.as_secs()));
        }
        let slice = NSDate::dateWithTimeIntervalSinceNow(0.1);
        // SAFETY: the default mode, a valid date.
        unsafe { run_loop.runMode_beforeDate(NSDefaultRunLoopMode, &slice) };
    };
    // SAFETY: as above.
    unsafe { manager.setDelegate(None) };
    answer
}
