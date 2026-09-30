//! Development TSF input DLL with an asynchronous Broker/librime backend.

#[cfg(windows)]
mod broker_client;
#[cfg(windows)]
mod broker_window;
#[cfg(windows)]
mod composition;
#[cfg(any(windows, test))]
mod composition_lease;
#[cfg(any(windows, test))]
mod edit_queue;
#[cfg(windows)]
mod key_adapter;
#[cfg(windows)]
mod runtime_adapter;
#[cfg(windows)]
mod windows_impl;
