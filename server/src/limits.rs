//! In-memory rate limiting (token buckets) and per-IP connection counting.
//! Keys are salted hashes of the client address; raw addresses are never stored or logged.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

struct Bucket {
    tokens: f64,
    last: Instant,
}

#[derive(Default)]
pub struct Limiter {
    buckets: Mutex<HashMap<String, Bucket>>,
    conns: Mutex<HashMap<String, usize>>,
}

impl Limiter {
    /// Take one token from bucket `key` (capacity `cap`, refilled at `per_sec`).
    pub fn check(&self, key: &str, cap: f64, per_sec: f64) -> bool {
        let mut m = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if m.len() > 50_000 {
            m.retain(|_, b| now.duration_since(b.last).as_secs() < 300);
        }
        let b = m.entry(key.to_string()).or_insert(Bucket { tokens: cap, last: now });
        let dt = now.duration_since(b.last).as_secs_f64();
        b.tokens = (b.tokens + dt * per_sec).min(cap);
        b.last = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    pub fn conn_inc(&self, ip: &str, max: usize) -> bool {
        let mut m = self.conns.lock().unwrap_or_else(|e| e.into_inner());
        let c = m.entry(ip.to_string()).or_insert(0);
        if *c >= max {
            return false;
        }
        *c += 1;
        true
    }
    pub fn conn_dec(&self, ip: &str) {
        let mut m = self.conns.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = m.get_mut(ip) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                m.remove(ip);
            }
        }
    }
}

/// Plain token bucket owned by one connection.
pub struct LocalBucket {
    tokens: f64,
    cap: f64,
    rate: f64,
    last: Instant,
}
impl LocalBucket {
    pub fn new(cap: f64, rate: f64) -> Self {
        LocalBucket { tokens: cap, cap, rate, last: Instant::now() }
    }
    pub fn take(&mut self) -> bool {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.last).as_secs_f64() * self.rate).min(self.cap);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

pub fn hash_ip(salt: &[u8; 16], ip: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt);
    h.update(ip.as_bytes());
    let d = h.finalize();
    d[..8].iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn ct_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
