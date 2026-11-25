//! Evaluating in the background, so the app never waits.

use std::{
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
};
use xpro_core::{
    eval::{Evaluator, Inputs},
    geometry::halve,
    graph::{Graph, NodeId},
    image::{Image, Srgb8Image},
    node::Ports as _,
    request::{Cancel, Cancelled, PreviewScale, Request},
    value::ValueType,
    view::{encode_srgb8, for_viewing},
};

/// How many bytes of values the evaluator keeps between evaluations. A full-size 24 megapixel
/// image is about 300 MB, and the default graph makes several.
const CACHE_BUDGET: usize = 3 << 30;

/// The preview scale evaluated first, for a quick look: a quarter of the full width and height.
const PREVIEW: PreviewScale = PreviewScale(2);

/// Levels stop being halved once they're this small on their longest side, which is about as
/// small as the window can show the image.
pub const SMALLEST_LEVEL: usize = 512;

/// What to show: a node's value, from a graph with its inputs bound.
pub struct Job {
    /// Identifies the job, counting up, so updates from older jobs can be told apart.
    pub generation: u64,
    pub graph: Graph,
    pub inputs: Inputs,
    pub node: NodeId,
    /// Levels bigger than this on either side are left out, since the GPU can't hold them.
    pub max_texture_side: usize,
}

/// Progress on a job.
pub enum Update {
    /// One size of the node's value, encoded for the screen.
    Level { generation: u64, level: Level },
    Failed {
        generation: u64,
        error: anyhow::Error,
    },
    /// The job is finished: every level has been sent, or it failed.
    Done { generation: u64 },
}

/// The node's value at one size.
pub struct Level {
    pub image: Srgb8Image,
    /// How many times the full-size image was halved to make this one.
    pub halvings: u8,
    /// The full-size image's width and height in pixels, which is only estimated until the
    /// full-size level arrives.
    pub full_size: [usize; 2],
}

/// A background thread that evaluates jobs, newest first.
pub struct Worker {
    pub(super) jobs: Option<mpsc::Sender<(Job, Cancel)>>,
    pub updates: mpsc::Receiver<Update>,
    /// Cancels the job being worked on.
    pub(super) cancel: Cancel,
    pub(super) thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// Starts the thread. `notify` is called after each update is sent.
    pub fn spawn(notify: impl Fn() + Send + 'static) -> Self {
        let (jobs, job_receiver) = mpsc::channel::<(Job, Cancel)>();
        let (update_sender, updates) = mpsc::channel();
        let thread = thread::spawn(move || {
            let mut evaluator = Evaluator::new(CACHE_BUDGET);
            while let Ok(mut job) = job_receiver.recv() {
                // Skip to the newest job. Older ones are already cancelled.
                while let Ok(newer) = job_receiver.try_recv() {
                    job = newer;
                }
                let (job, cancel) = job;
                let send = |update| {
                    let sent = update_sender.send(update).is_ok();
                    notify();
                    sent
                };
                if !work(&mut evaluator, &job, &cancel, &send) {
                    return;
                }
            }
        });
        Self {
            jobs: Some(jobs),
            updates,
            cancel: Cancel::new(),
            thread: Some(thread),
        }
    }

    /// Cancels the current job, and starts on this one.
    pub fn submit(&mut self, job: Job) {
        self.cancel.cancel();
        self.cancel = Cancel::new();
        if let Some(jobs) = &self.jobs {
            // If the thread has stopped, the app will see the updates channel close.
            let _ = jobs.send((job, self.cancel.clone()));
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.cancel();
        // Closing the channel ends the thread's loop.
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Works on a job, sending levels as they're ready: a quick preview first, then full size.
/// Returns false if nobody's listening anymore.
fn work(
    evaluator: &mut Evaluator,
    job: &Job,
    cancel: &Cancel,
    send: &impl Fn(Update) -> bool,
) -> bool {
    let generation = job.generation;
    let result = (|| -> anyhow::Result<bool> {
        // A mosaic is always full size, so there's no quicker preview of one.
        let node = &job.graph.get(&job.node).map(|node| &node.node);
        let full_size_only = node.is_some_and(|node| node.output_type() == ValueType::Mosaic);
        let first = if full_size_only {
            PreviewScale::FULL
        } else {
            PREVIEW
        };
        let preview = view(evaluator, job, first, cancel)?;
        let full_size = estimate_full_size(&preview, first);
        if !send_levels(job, preview, first.halvings(), u8::MAX, full_size, send) {
            return Ok(false);
        }
        if first != PreviewScale::FULL {
            let full = view(evaluator, job, PreviewScale::FULL, cancel)?;
            let full_size = [full.width, full.height];
            // Smaller levels came from the preview already.
            if !send_levels(job, full, 0, first.halvings(), full_size, send) {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    match result {
        Ok(listening) => listening && send(Update::Done { generation }),
        Err(error) if Cancelled::caused(&error) => true,
        Err(error) => {
            send(Update::Failed { generation, error }) && send(Update::Done { generation })
        }
    }
}

/// Evaluates the job's node at a scale, as an image to look at.
fn view(
    evaluator: &mut Evaluator,
    job: &Job,
    scale: PreviewScale,
    cancel: &Cancel,
) -> anyhow::Result<Arc<Image>> {
    let request = Request {
        scale,
        cancel: cancel.clone(),
    };
    let value = evaluator.evaluate(&job.graph, &job.inputs, &job.node, &request)?;
    cancel.check()?;
    for_viewing(&value)
}

/// The full size of an image, given a preview of it halved `scale` times.
fn estimate_full_size(preview: &Image, scale: PreviewScale) -> [usize; 2] {
    let factor = 1 << scale.halvings();
    [preview.width * factor, preview.height * factor]
}

/// Sends `image`, which is the full-size image halved `halvings` times, then halvings of it,
/// down to [`SMALLEST_LEVEL`] or to `until` halvings, whichever comes first. Returns false if
/// nobody's listening anymore.
fn send_levels(
    job: &Job,
    mut image: Arc<Image>,
    mut halvings: u8,
    until: u8,
    full_size: [usize; 2],
    send: &impl Fn(Update) -> bool,
) -> bool {
    while halvings < until {
        let longest = image.width.max(image.height);
        if longest <= job.max_texture_side {
            let level = match encode_srgb8(&image) {
                Ok(encoded) => Level {
                    image: encoded,
                    halvings,
                    full_size,
                },
                Err(error) => {
                    return send(Update::Failed {
                        generation: job.generation,
                        error,
                    });
                }
            };
            if !send(Update::Level {
                generation: job.generation,
                level,
            }) {
                return false;
            }
        }
        if longest <= SMALLEST_LEVEL {
            break;
        }
        image = Arc::new(halve(&image));
        halvings += 1;
    }
    true
}
