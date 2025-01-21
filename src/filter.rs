use crate::{bail_ffmpeg, cstr, rstr, set_opts};
use anyhow::Error;
use ffmpeg_sys_the_third::{
    av_strdup, avfilter_get_by_name, avfilter_graph_alloc, avfilter_graph_alloc_filter,
    avfilter_graph_config, avfilter_graph_create_filter, avfilter_graph_dump, avfilter_graph_parse,
    avfilter_inout_alloc, AVFilterContext, AVFilterGraph, AVFrame,
};
use log::debug;
use std::collections::HashMap;
use std::ptr;

pub struct Filter {
    graph: *mut AVFilterGraph,
}

impl Default for Filter {
    fn default() -> Self {
        Self::new()
    }
}

impl Filter {
    pub fn new() -> Self {
        Self {
            graph: unsafe { avfilter_graph_alloc() },
        }
    }

    /// Parse filter from string using [avfilter_graph_parse2]
    ///
    /// https://ffmpeg.org/ffmpeg-filters.html
    pub unsafe fn parse(graph: &str) -> Result<Self, Error> {
        let ctx = avfilter_graph_alloc();
        let inputs = avfilter_inout_alloc();
        let outputs = avfilter_inout_alloc();
        let src = avfilter_get_by_name(cstr!("buffer"));
        let dst = avfilter_get_by_name(cstr!("buffersink"));
        let mut src_ctx = ptr::null_mut();
        let mut dst_ctx = ptr::null_mut();
        let ret = avfilter_graph_create_filter(
            &mut src_ctx,
            src,
            cstr!("in"),
            ptr::null_mut(),
            ptr::null_mut(),
            ctx,
        );
        bail_ffmpeg!(ret, "Failed to parse graph");

        let ret = avfilter_graph_create_filter(
            &mut dst_ctx,
            dst,
            cstr!("out"),
            ptr::null_mut(),
            ptr::null_mut(),
            ctx,
        );
        bail_ffmpeg!(ret, "Failed to parse graph");

        (*outputs).name = av_strdup((*dst).name);
        (*outputs).filter_ctx = dst_ctx;
        (*outputs).pad_idx = 0;
        (*outputs).next = ptr::null_mut();

        (*inputs).name = av_strdup((*src).name);
        (*inputs).filter_ctx = src_ctx;
        (*inputs).pad_idx = 0;
        (*inputs).next = ptr::null_mut();

        let ret = avfilter_graph_parse(ctx, cstr!(graph), inputs, outputs, ptr::null_mut());
        bail_ffmpeg!(ret, "Failed to parse graph");
        let mut ret = Self { graph: ctx };
        ret.build()?;
        Ok(ret)
    }

    pub fn add_filter(
        &mut self,
        name: &str,
        options: Option<HashMap<String, String>>,
    ) -> Result<*mut AVFilterContext, Error> {
        if self.graph.is_null() {
            anyhow::bail!("Filter graph is null.");
        }
        unsafe {
            let filter = avfilter_get_by_name(cstr!(name));
            if filter.is_null() {
                anyhow::bail!("Filter {} not found", name);
            }
            let flt = avfilter_graph_alloc_filter(self.graph, filter, ptr::null_mut());
            if flt.is_null() {
                anyhow::bail!("Filter {} not found", name);
            }
            if let Some(opt) = options {
                set_opts(flt as *mut libc::c_void, opt)?;
            }
            Ok(flt)
        }
    }

    pub unsafe fn build(&mut self) -> Result<(), Error> {
        let d = rstr!(avfilter_graph_dump(self.graph, ptr::null_mut()));
        debug!("{}", d);

        let ret = avfilter_graph_config(self.graph, ptr::null_mut());
        bail_ffmpeg!(ret, "Failed to build filter");
        Ok(())
    }

    pub unsafe fn process_frame(&mut self, _frame: *mut AVFrame) -> Result<*mut AVFrame, Error> {
        todo!();
    }
}
