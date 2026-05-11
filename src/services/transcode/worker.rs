use std::sync::mpsc::{Receiver, Sender};

pub struct TranscodeWorker {
    task_queue: Receiver<String>,
    queue: Sender<String>,
}
// new thread for transcoding, will be called by the controller when a new file is uploaded
impl TranscodeWorker {
    pub fn new(task_queue: Receiver<String>, queue: Sender<String>) -> Self {
        TranscodeWorker { task_queue, queue }
    }

    pub fn start(&self) {
        while let Ok(file_path) = self.task_queue.recv() {
            println!("Received file for transcoding: {}", file_path);
            // Simulate transcoding process
            std::thread::sleep(std::time::Duration::from_secs(5));
            println!("Finished transcoding: {}", file_path);
            // Send the transcoded file path back to the controller
            self.queue.send(file_path).unwrap();
        }
    }
}
