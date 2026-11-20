//! The raster helper process: decodes and rasterizes plugin images for the launcher, one
//! job at a time over stdin and stdout (see `tomakast_raster::protocol`). The launcher
//! starts it with limits on its memory and CPU time, and stops it when a job runs late.

fn main() {
    tomakast_raster::serve();
}
