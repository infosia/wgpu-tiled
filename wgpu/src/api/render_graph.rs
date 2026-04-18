use alloc::{string::String, vec, vec::Vec};

use crate::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttachmentRole {
    Color,
    Depth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttachmentLifetime {
    Transient,
    Persistent,
}

#[derive(Clone, Debug)]
struct AttachmentSpec {
    label: String,
    format: TextureFormat,
    role: AttachmentRole,
    lifetime: AttachmentLifetime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadKind {
    Color,
    Depth,
}

#[derive(Clone, Copy, Debug)]
struct ReadSpec {
    attachment: AttachmentId,
    kind: ReadKind,
}

#[derive(Clone, Debug)]
struct SubpassSpec {
    label: String,
    writes_color: Vec<AttachmentId>,
    writes_depth: Vec<AttachmentId>,
    reads: Vec<ReadSpec>,
}

impl SubpassSpec {
    fn new(label: &str) -> Self {
        Self {
            label: label.into(),
            writes_color: Vec::new(),
            writes_depth: Vec::new(),
            reads: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ResolvedRead {
    source_subpass: SubpassIndex,
}

#[derive(Clone, Debug, Default)]
struct BuiltSubpass {
    reads: Vec<ResolvedRead>,
    color_attachment_indices: Vec<u32>,
    uses_depth_stencil: bool,
    input_attachments: Vec<SubpassInputAttachment>,
}

/// Opaque identifier returned by [`RenderGraphBuilder`] when registering an attachment.
///
/// The inner numeric value is intentionally private.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AttachmentId(u32);
#[cfg(send_sync)]
static_assertions::assert_impl_all!(AttachmentId: Send, Sync);

/// Error returned when building a [`RenderGraph`] from [`RenderGraphBuilder`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderGraphError {
    /// The graph has no subpasses.
    NoSubpasses,
    /// The requested sample count is invalid.
    InvalidSampleCount {
        /// Invalid sample count.
        sample_count: u32,
    },
    /// The graph exceeds [`ActiveSubpassMask::MAX_SUBPASSES`].
    TooManySubpasses {
        /// Number of declared subpasses.
        count: u32,
        /// Maximum supported number of subpasses.
        maximum: u32,
    },
    /// A referenced attachment id is not part of the graph.
    InvalidAttachmentId {
        /// Invalid attachment id.
        attachment: AttachmentId,
    },
    /// An attachment was used in the wrong role.
    AttachmentRoleMismatch {
        /// Attachment id with the mismatch.
        attachment: AttachmentId,
        /// Expected role (`"color"` or `"depth"`).
        expected: &'static str,
        /// Actual role (`"color"` or `"depth"`).
        found: &'static str,
    },
    /// A read references an attachment that has no earlier writer.
    ReadBeforeWrite {
        /// Subpass that performed the read.
        subpass: SubpassIndex,
        /// Attachment that was read.
        attachment: AttachmentId,
    },
    /// A persistent attachment has no writer in any subpass.
    PersistentNeverWritten {
        /// Persistent attachment missing a writer.
        attachment: AttachmentId,
    },
    /// A subpass declares more than one depth write attachment.
    MultipleDepthWrites {
        /// Subpass that declares multiple depth writes.
        subpass: SubpassIndex,
    },
    /// A subpass declares more than one depth input attachment.
    MultipleDepthReads {
        /// Subpass that declares multiple depth reads.
        subpass: SubpassIndex,
    },
    /// A subpass writes the same attachment more than once.
    DuplicateAttachmentWrite {
        /// Subpass with the duplicate write.
        subpass: SubpassIndex,
        /// Attachment written multiple times.
        attachment: AttachmentId,
    },
}

/// Error returned by [`RenderGraph::resolve_active`] when a culling mask is invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubpassCullError {
    /// One entry in the `active` list is outside the graph's subpass range.
    InvalidSubpassIndex {
        /// Out-of-range subpass index.
        subpass: SubpassIndex,
        /// Total number of subpasses in the graph.
        subpass_count: u32,
    },
    /// An active subpass reads from a culled producer subpass.
    ActiveReadsFromCulled {
        /// Active subpass index.
        active: SubpassIndex,
        /// Culled producer subpass index.
        culled: SubpassIndex,
    },
    /// Culling removes all active writers for a persistent output attachment.
    PersistentOutputCulled {
        /// Persistent attachment that lost all active writers.
        attachment: AttachmentId,
    },
}

/// Declarative builder for multi-subpass render graphs.
#[derive(Debug, Default)]
pub struct RenderGraphBuilder {
    sample_count: u32,
    attachments: Vec<AttachmentSpec>,
    subpasses: Vec<SubpassSpec>,
}
#[cfg(send_sync)]
static_assertions::assert_impl_all!(RenderGraphBuilder: Send, Sync);

impl RenderGraphBuilder {
    /// Creates a new builder with `sample_count = 1`.
    pub fn new() -> Self {
        Self {
            sample_count: 1,
            ..Default::default()
        }
    }

    /// Sets the sample count used by all generated subpass layouts.
    pub fn sample_count(&mut self, n: u32) -> &mut Self {
        self.sample_count = n;
        self
    }

    /// Registers a transient color attachment.
    pub fn add_transient_color(&mut self, label: &str, format: TextureFormat) -> AttachmentId {
        self.push_attachment(
            label,
            format,
            AttachmentRole::Color,
            AttachmentLifetime::Transient,
        )
    }

    /// Registers a transient depth/stencil attachment.
    pub fn add_transient_depth(&mut self, label: &str, format: TextureFormat) -> AttachmentId {
        self.push_attachment(
            label,
            format,
            AttachmentRole::Depth,
            AttachmentLifetime::Transient,
        )
    }

    /// Registers a persistent color attachment.
    pub fn add_persistent_color(&mut self, label: &str, format: TextureFormat) -> AttachmentId {
        self.push_attachment(
            label,
            format,
            AttachmentRole::Color,
            AttachmentLifetime::Persistent,
        )
    }

    /// Registers a persistent depth/stencil attachment.
    pub fn add_persistent_depth(&mut self, label: &str, format: TextureFormat) -> AttachmentId {
        self.push_attachment(
            label,
            format,
            AttachmentRole::Depth,
            AttachmentLifetime::Persistent,
        )
    }

    fn push_attachment(
        &mut self,
        label: &str,
        format: TextureFormat,
        role: AttachmentRole,
        lifetime: AttachmentLifetime,
    ) -> AttachmentId {
        let id = AttachmentId(self.attachments.len() as u32);
        self.attachments.push(AttachmentSpec {
            label: label.into(),
            format,
            role,
            lifetime,
        });
        id
    }

    /// Adds a subpass and returns a fluent subpass builder for it.
    pub fn add_subpass(&mut self, label: &str) -> SubpassBuilder<'_> {
        let subpass = self.subpasses.len();
        self.subpasses.push(SubpassSpec::new(label));
        SubpassBuilder {
            graph: self,
            subpass,
        }
    }

    /// Builds an immutable render graph.
    pub fn build(self) -> Result<RenderGraph, RenderGraphError> {
        if self.sample_count == 0 {
            return Err(RenderGraphError::InvalidSampleCount { sample_count: 0 });
        }
        if self.subpasses.is_empty() {
            return Err(RenderGraphError::NoSubpasses);
        }

        let subpass_count = self.subpasses.len() as u32;
        if subpass_count > ActiveSubpassMask::MAX_SUBPASSES {
            return Err(RenderGraphError::TooManySubpasses {
                count: subpass_count,
                maximum: ActiveSubpassMask::MAX_SUBPASSES,
            });
        }

        let attachment_labels = self
            .attachments
            .iter()
            .map(|attachment| attachment.label.clone())
            .collect::<Vec<_>>();
        let subpass_labels = self
            .subpasses
            .iter()
            .map(|subpass| subpass.label.clone())
            .collect::<Vec<_>>();

        let mut color_attachment_indices = vec![None; self.attachments.len()];
        let mut next_color_attachment_index = 0u32;
        for (attachment_index, attachment) in self.attachments.iter().enumerate() {
            if attachment.role == AttachmentRole::Color {
                color_attachment_indices[attachment_index] = Some(next_color_attachment_index);
                next_color_attachment_index = next_color_attachment_index.saturating_add(1);
            }
        }

        let mut dependencies = Vec::new();
        let mut subpass_layouts = Vec::with_capacity(self.subpasses.len());
        let mut built_subpasses = Vec::with_capacity(self.subpasses.len());
        let mut last_color_writer = vec![None; self.attachments.len()];
        let mut last_depth_writer = vec![None; self.attachments.len()];
        let mut writer_subpasses = vec![Vec::new(); self.attachments.len()];
        let mut persistent_outputs = Vec::new();

        for (attachment_index, attachment) in self.attachments.iter().enumerate() {
            if attachment.lifetime == AttachmentLifetime::Persistent {
                persistent_outputs.push(AttachmentId(attachment_index as u32));
            }
        }

        for (subpass_index, subpass) in self.subpasses.iter().enumerate() {
            if subpass.writes_depth.len() > 1 {
                return Err(RenderGraphError::MultipleDepthWrites {
                    subpass: SubpassIndex(subpass_index as u32),
                });
            }
            let depth_reads = subpass
                .reads
                .iter()
                .filter(|read| read.kind == ReadKind::Depth)
                .count();
            if depth_reads > 1 {
                return Err(RenderGraphError::MultipleDepthReads {
                    subpass: SubpassIndex(subpass_index as u32),
                });
            }

            let mut color_formats = Vec::with_capacity(subpass.writes_color.len());
            let mut input_attachment_formats = Vec::with_capacity(subpass.reads.len());
            let mut depth_stencil_format = None;
            let mut built = BuiltSubpass::default();
            let mut seen_color_writes = Vec::with_capacity(subpass.writes_color.len());

            for (slot, attachment_id) in subpass.writes_color.iter().copied().enumerate() {
                if seen_color_writes.contains(&attachment_id) {
                    return Err(RenderGraphError::DuplicateAttachmentWrite {
                        subpass: SubpassIndex(subpass_index as u32),
                        attachment: attachment_id,
                    });
                }
                seen_color_writes.push(attachment_id);
                let attachment = get_attachment(&self.attachments, attachment_id)?;
                ensure_role(attachment, attachment_id, AttachmentRole::Color)?;
                color_formats.push(Some(attachment.format));
                writer_subpasses[attachment_id.0 as usize].push(SubpassIndex(subpass_index as u32));
                last_color_writer[attachment_id.0 as usize] =
                    Some((SubpassIndex(subpass_index as u32), slot as u32));
                let color_attachment_index = color_attachment_indices[attachment_id.0 as usize]
                    .expect("color attachment index is available for color attachments");
                built.color_attachment_indices.push(color_attachment_index);
            }

            if let Some(attachment_id) = subpass.writes_depth.first().copied() {
                let attachment = get_attachment(&self.attachments, attachment_id)?;
                ensure_role(attachment, attachment_id, AttachmentRole::Depth)?;
                depth_stencil_format = Some(attachment.format);
                writer_subpasses[attachment_id.0 as usize].push(SubpassIndex(subpass_index as u32));
                last_depth_writer[attachment_id.0 as usize] =
                    Some(SubpassIndex(subpass_index as u32));
                built.uses_depth_stencil = true;
            }

            for read in subpass.reads.iter().copied() {
                let attachment = get_attachment(&self.attachments, read.attachment)?;
                let expected_role = match read.kind {
                    ReadKind::Color => AttachmentRole::Color,
                    ReadKind::Depth => AttachmentRole::Depth,
                };
                ensure_role(attachment, read.attachment, expected_role)?;

                let source = match read.kind {
                    ReadKind::Color => {
                        let Some((source_subpass, attachment_index)) =
                            last_color_writer[read.attachment.0 as usize]
                        else {
                            return Err(RenderGraphError::ReadBeforeWrite {
                                subpass: SubpassIndex(subpass_index as u32),
                                attachment: read.attachment,
                            });
                        };
                        upsert_dependency(
                            &mut dependencies,
                            source_subpass,
                            SubpassIndex(subpass_index as u32),
                            SubpassDependencyType::ColorToInput,
                        );
                        SubpassInputSource::Color {
                            subpass: source_subpass,
                            attachment_index,
                        }
                    }
                    ReadKind::Depth => {
                        let Some(source_subpass) = last_depth_writer[read.attachment.0 as usize]
                        else {
                            return Err(RenderGraphError::ReadBeforeWrite {
                                subpass: SubpassIndex(subpass_index as u32),
                                attachment: read.attachment,
                            });
                        };
                        upsert_dependency(
                            &mut dependencies,
                            source_subpass,
                            SubpassIndex(subpass_index as u32),
                            SubpassDependencyType::DepthToInput,
                        );
                        SubpassInputSource::Depth {
                            subpass: source_subpass,
                        }
                    }
                };

                built.reads.push(ResolvedRead {
                    source_subpass: match source {
                        SubpassInputSource::Color { subpass, .. }
                        | SubpassInputSource::Depth { subpass } => subpass,
                    },
                });
                built.input_attachments.push(SubpassInputAttachment {
                    binding: built.input_attachments.len() as u32,
                    source,
                });
                input_attachment_formats.push(attachment.format);
            }

            if depth_stencil_format.is_none() {
                depth_stencil_format = subpass
                    .reads
                    .iter()
                    .find(|read| read.kind == ReadKind::Depth)
                    .map(|read| self.attachments[read.attachment.0 as usize].format);
                if depth_stencil_format.is_some() {
                    built.uses_depth_stencil = true;
                }
            }

            subpass_layouts.push(SubpassLayout {
                color_formats,
                depth_stencil_format,
                sample_count: self.sample_count,
                input_attachment_formats,
            });
            built_subpasses.push(built);
        }

        for attachment in persistent_outputs.iter().copied() {
            if writer_subpasses[attachment.0 as usize].is_empty() {
                return Err(RenderGraphError::PersistentNeverWritten { attachment });
            }
        }

        Ok(RenderGraph {
            subpass_count,
            subpass_layouts,
            dependencies,
            sample_count: self.sample_count,
            subpasses: built_subpasses,
            writer_subpasses,
            persistent_outputs,
            attachment_labels,
            subpass_labels,
        })
    }
}

fn get_attachment(
    attachments: &[AttachmentSpec],
    attachment_id: AttachmentId,
) -> Result<&AttachmentSpec, RenderGraphError> {
    attachments
        .get(attachment_id.0 as usize)
        .ok_or(RenderGraphError::InvalidAttachmentId {
            attachment: attachment_id,
        })
}

fn role_name(role: AttachmentRole) -> &'static str {
    match role {
        AttachmentRole::Color => "color",
        AttachmentRole::Depth => "depth",
    }
}

fn ensure_role(
    attachment: &AttachmentSpec,
    attachment_id: AttachmentId,
    expected: AttachmentRole,
) -> Result<(), RenderGraphError> {
    if attachment.role == expected {
        return Ok(());
    }
    Err(RenderGraphError::AttachmentRoleMismatch {
        attachment: attachment_id,
        expected: role_name(expected),
        found: role_name(attachment.role),
    })
}

fn merge_dependency_type(
    a: SubpassDependencyType,
    b: SubpassDependencyType,
) -> SubpassDependencyType {
    if a == b {
        return a;
    }
    SubpassDependencyType::ColorDepthToInput
}

fn upsert_dependency(
    dependencies: &mut Vec<SubpassDependency>,
    src_subpass: SubpassIndex,
    dst_subpass: SubpassIndex,
    dependency_type: SubpassDependencyType,
) {
    if let Some(existing) = dependencies.iter_mut().find(|dependency| {
        dependency.src_subpass == src_subpass && dependency.dst_subpass == dst_subpass
    }) {
        existing.dependency_type = merge_dependency_type(existing.dependency_type, dependency_type);
        return;
    }

    dependencies.push(SubpassDependency {
        src_subpass,
        dst_subpass,
        dependency_type,
        by_region: true,
    });
}

/// Fluent builder used to define one subpass in a [`RenderGraphBuilder`].
#[derive(Debug)]
pub struct SubpassBuilder<'g> {
    graph: &'g mut RenderGraphBuilder,
    subpass: usize,
}

impl SubpassBuilder<'_> {
    /// Marks a color attachment as written by this subpass.
    pub fn writes_color(self, id: AttachmentId) -> Self {
        self.graph.subpasses[self.subpass].writes_color.push(id);
        self
    }

    /// Marks a depth attachment as written by this subpass.
    pub fn writes_depth(self, id: AttachmentId) -> Self {
        self.graph.subpasses[self.subpass].writes_depth.push(id);
        self
    }

    /// Adds a color input attachment read.
    pub fn reads(self, id: AttachmentId) -> Self {
        self.graph.subpasses[self.subpass].reads.push(ReadSpec {
            attachment: id,
            kind: ReadKind::Color,
        });
        self
    }

    /// Adds a depth input attachment read.
    pub fn reads_depth(self, id: AttachmentId) -> Self {
        self.graph.subpasses[self.subpass].reads.push(ReadSpec {
            attachment: id,
            kind: ReadKind::Depth,
        });
        self
    }
}

/// Immutable render graph produced by [`RenderGraphBuilder`].
#[derive(Clone, Debug)]
pub struct RenderGraph {
    /// Number of subpasses in this graph.
    pub subpass_count: u32,
    /// Per-subpass compatibility metadata.
    pub subpass_layouts: Vec<SubpassLayout>,
    /// Auto-generated dependencies between subpasses.
    pub dependencies: Vec<SubpassDependency>,
    /// Sample count propagated to all subpass layouts.
    pub sample_count: u32,
    subpasses: Vec<BuiltSubpass>,
    writer_subpasses: Vec<Vec<SubpassIndex>>,
    persistent_outputs: Vec<AttachmentId>,
    attachment_labels: Vec<String>,
    subpass_labels: Vec<String>,
}
#[cfg(send_sync)]
static_assertions::assert_impl_all!(RenderGraph: Send, Sync);

/// Borrowed descriptor views derived from a [`RenderGraph`].
///
/// This bridges the graph's immutable representation into the attachment-index
/// structures expected by [`RenderPassDescriptor`].
#[derive(Debug)]
pub struct RenderGraphDescriptorViews<'a> {
    /// Subpass descriptors for [`RenderPassDescriptor::subpasses`].
    pub subpasses: Vec<SubpassDescriptor<'a>>,
    /// Dependencies for [`RenderPassDescriptor::subpass_dependencies`].
    pub subpass_dependencies: &'a [SubpassDependency],
    /// Compatibility layouts generated at build-time.
    pub subpass_layouts: &'a [SubpassLayout],
}
#[cfg(send_sync)]
static_assertions::assert_impl_all!(RenderGraphDescriptorViews<'_>: Send, Sync);

impl RenderGraph {
    /// Returns borrowed subpass/dependency/layout views suitable for a render pass descriptor.
    pub fn descriptor_views(&self) -> RenderGraphDescriptorViews<'_> {
        RenderGraphDescriptorViews {
            subpasses: self
                .subpasses
                .iter()
                .map(|subpass| SubpassDescriptor {
                    color_attachment_indices: &subpass.color_attachment_indices,
                    uses_depth_stencil: subpass.uses_depth_stencil,
                    input_attachments: &subpass.input_attachments,
                })
                .collect(),
            subpass_dependencies: &self.dependencies,
            subpass_layouts: &self.subpass_layouts,
        }
    }

    /// Builds a [`SubpassTarget`] compatible with this graph.
    pub fn make_subpass_target(
        &self,
        index: u32,
        color_attachment_formats: &[Option<TextureFormat>],
        depth_stencil_format: Option<TextureFormat>,
    ) -> SubpassTarget {
        let subpass_descs = self
            .subpasses
            .iter()
            .map(|subpass| SubpassTargetDesc {
                color_attachment_indices: subpass.color_attachment_indices.clone(),
                uses_depth_stencil: subpass.uses_depth_stencil,
                input_attachment_indices: subpass
                    .input_attachments
                    .iter()
                    .map(|input| match input.source {
                        SubpassInputSource::Color { subpass, .. }
                        | SubpassInputSource::Depth { subpass } => subpass.0,
                    })
                    .collect(),
            })
            .collect();

        SubpassTarget {
            index,
            color_attachment_formats: color_attachment_formats.to_vec(),
            depth_stencil_format,
            subpass_descs,
            dependencies: self.dependencies.clone(),
        }
    }

    /// Returns the label associated with an attachment id.
    pub fn attachment_label(&self, attachment: AttachmentId) -> Option<&str> {
        self.attachment_labels
            .get(attachment.0 as usize)
            .map(String::as_str)
    }

    /// Returns the label associated with a subpass index.
    pub fn subpass_label(&self, subpass: SubpassIndex) -> Option<&str> {
        self.subpass_labels
            .get(subpass.0 as usize)
            .map(String::as_str)
    }

    /// Resolves a list of active subpasses into an [`ActiveSubpassMask`].
    ///
    /// Validation rules:
    /// - Active subpasses must not read from culled producer subpasses.
    /// - Every persistent output attachment must keep at least one active writer.
    pub fn resolve_active(
        &self,
        active: &[SubpassIndex],
    ) -> Result<ActiveSubpassMask, SubpassCullError> {
        let mut mask = ActiveSubpassMask::NONE;
        for &subpass in active {
            if subpass.0 >= self.subpass_count {
                return Err(SubpassCullError::InvalidSubpassIndex {
                    subpass,
                    subpass_count: self.subpass_count,
                });
            }
            mask = mask.with(subpass);
        }

        for (active_subpass_index, subpass) in self.subpasses.iter().enumerate() {
            let active_subpass = SubpassIndex(active_subpass_index as u32);
            if !mask.is_active(active_subpass) {
                continue;
            }
            for read in &subpass.reads {
                if !mask.is_active(read.source_subpass) {
                    return Err(SubpassCullError::ActiveReadsFromCulled {
                        active: active_subpass,
                        culled: read.source_subpass,
                    });
                }
            }
        }

        for persistent in self.persistent_outputs.iter().copied() {
            let has_active_writer = self.writer_subpasses[persistent.0 as usize]
                .iter()
                .copied()
                .any(|writer| mask.is_active(writer));
            if !has_active_writer {
                return Err(SubpassCullError::PersistentOutputCulled {
                    attachment: persistent,
                });
            }
        }

        Ok(mask)
    }
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<RenderGraph>();
};

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(attachment_id: u32) -> AttachmentId {
        AttachmentId(attachment_id)
    }

    #[test]
    fn build_without_subpasses_fails() {
        let mut builder = RenderGraphBuilder::new();
        let _ = builder.add_transient_color("a", TextureFormat::Rgba8Unorm);
        let err = builder.build().unwrap_err();
        assert!(matches!(err, RenderGraphError::NoSubpasses));
    }

    #[test]
    fn build_read_before_write_fails() {
        let mut builder = RenderGraphBuilder::new();
        let albedo = builder.add_transient_color("albedo", TextureFormat::Rgba8Unorm);
        let _ = builder.add_subpass("lighting").reads(albedo);
        let err = builder.build().unwrap_err();
        assert!(matches!(
            err,
            RenderGraphError::ReadBeforeWrite {
                subpass: SubpassIndex(0),
                attachment,
            } if attachment == albedo
        ));
    }

    #[test]
    fn build_persistent_never_written_fails() {
        let mut builder = RenderGraphBuilder::new();
        let transient = builder.add_transient_color("transient", TextureFormat::Rgba8Unorm);
        let persistent = builder.add_persistent_color("out", TextureFormat::Rgba8Unorm);
        let _ = builder.add_subpass("pass").writes_color(transient);
        let err = builder.build().unwrap_err();
        assert!(matches!(
            err,
            RenderGraphError::PersistentNeverWritten { attachment } if attachment == persistent
        ));
    }

    #[test]
    fn build_two_subpasses_generates_expected_dependency() {
        let mut builder = RenderGraphBuilder::new();
        let albedo = builder.add_transient_color("albedo", TextureFormat::Rgba8Unorm);
        let _ = builder.add_subpass("gbuffer").writes_color(albedo);
        let _ = builder.add_subpass("lighting").reads(albedo);

        let graph = builder.build().unwrap();
        assert_eq!(graph.dependencies.len(), 1);
        assert_eq!(
            graph.dependencies[0],
            SubpassDependency {
                src_subpass: SubpassIndex(0),
                dst_subpass: SubpassIndex(1),
                dependency_type: SubpassDependencyType::ColorToInput,
                by_region: true,
            }
        );
    }

    #[test]
    fn build_three_subpasses_generates_by_region_dependencies() {
        let mut builder = RenderGraphBuilder::new();
        let gbuffer = builder.add_transient_color("gbuffer", TextureFormat::Rgba8Unorm);
        let hdr = builder.add_transient_color("hdr", TextureFormat::Rgba16Float);
        let out = builder.add_persistent_color("out", TextureFormat::Rgba8Unorm);
        let _ = builder.add_subpass("gbuffer").writes_color(gbuffer);
        let _ = builder
            .add_subpass("lighting")
            .reads(gbuffer)
            .writes_color(hdr);
        let _ = builder
            .add_subpass("composite")
            .reads(hdr)
            .writes_color(out);

        let graph = builder.build().unwrap();
        assert_eq!(graph.dependencies.len(), 2);
        assert!(graph
            .dependencies
            .iter()
            .all(|dependency| dependency.by_region));
        assert_eq!(graph.dependencies[0].src_subpass, SubpassIndex(0));
        assert_eq!(graph.dependencies[0].dst_subpass, SubpassIndex(1));
        assert_eq!(graph.dependencies[1].src_subpass, SubpassIndex(1));
        assert_eq!(graph.dependencies[1].dst_subpass, SubpassIndex(2));
    }

    #[test]
    fn sample_count_propagates_to_layouts() {
        let mut builder = RenderGraphBuilder::new();
        builder.sample_count(4);
        let albedo = builder.add_transient_color("albedo", TextureFormat::Rgba8Unorm);
        let _ = builder.add_subpass("gbuffer").writes_color(albedo);

        let graph = builder.build().unwrap();
        assert_eq!(graph.sample_count, 4);
        assert_eq!(graph.subpass_layouts.len(), 1);
        assert_eq!(graph.subpass_layouts[0].sample_count, 4);
    }

    #[test]
    fn resolve_active_accepts_empty_and_partial_masks_when_valid() {
        let mut builder = RenderGraphBuilder::new();
        let a = builder.add_transient_color("a", TextureFormat::Rgba8Unorm);
        let b = builder.add_transient_color("b", TextureFormat::Rgba8Unorm);
        let c = builder.add_transient_color("c", TextureFormat::Rgba8Unorm);
        let _ = builder.add_subpass("s0").writes_color(a);
        let _ = builder.add_subpass("s1").writes_color(b);
        let _ = builder.add_subpass("s2").writes_color(c);
        let graph = builder.build().unwrap();

        let none_active = graph.resolve_active(&[]).unwrap();
        assert!(!none_active.is_active(SubpassIndex(0)));
        assert!(!none_active.is_active(SubpassIndex(1)));
        assert!(!none_active.is_active(SubpassIndex(2)));

        let partial = graph
            .resolve_active(&[SubpassIndex(0), SubpassIndex(2)])
            .unwrap();
        assert!(partial.is_active(SubpassIndex(0)));
        assert!(!partial.is_active(SubpassIndex(1)));
        assert!(partial.is_active(SubpassIndex(2)));
    }

    #[test]
    fn resolve_active_rejects_active_reads_from_culled() {
        let mut builder = RenderGraphBuilder::new();
        let gbuffer = builder.add_transient_color("gbuffer", TextureFormat::Rgba8Unorm);
        let hdr = builder.add_transient_color("hdr", TextureFormat::Rgba16Float);
        let _ = builder.add_subpass("gbuffer").writes_color(gbuffer);
        let _ = builder
            .add_subpass("lighting")
            .reads(gbuffer)
            .writes_color(hdr);
        let _ = builder.add_subpass("composite").reads(hdr);
        let graph = builder.build().unwrap();

        let err = graph
            .resolve_active(&[SubpassIndex(0), SubpassIndex(2)])
            .unwrap_err();
        assert!(matches!(
            err,
            SubpassCullError::ActiveReadsFromCulled {
                active: SubpassIndex(2),
                culled: SubpassIndex(1),
            }
        ));
    }

    #[test]
    fn resolve_active_rejects_culling_last_persistent_writer() {
        let mut builder = RenderGraphBuilder::new();
        let gbuffer = builder.add_transient_color("gbuffer", TextureFormat::Rgba8Unorm);
        let out = builder.add_persistent_color("out", TextureFormat::Rgba8Unorm);
        let _ = builder.add_subpass("gbuffer").writes_color(gbuffer);
        let _ = builder.add_subpass("composite").writes_color(out);
        let graph = builder.build().unwrap();

        let err = graph.resolve_active(&[SubpassIndex(0)]).unwrap_err();
        assert!(matches!(
            err,
            SubpassCullError::PersistentOutputCulled { attachment } if attachment == out
        ));
    }

    #[test]
    fn resolve_active_rejects_invalid_subpass_index() {
        let mut builder = RenderGraphBuilder::new();
        let color = builder.add_transient_color("color", TextureFormat::Rgba8Unorm);
        let _ = builder.add_subpass("s0").writes_color(color);
        let graph = builder.build().unwrap();

        let err = graph.resolve_active(&[SubpassIndex(9)]).unwrap_err();
        assert!(matches!(
            err,
            SubpassCullError::InvalidSubpassIndex {
                subpass: SubpassIndex(9),
                subpass_count: 1
            }
        ));
    }

    #[test]
    fn resolve_active_accepts_all_subpasses() {
        let mut builder = RenderGraphBuilder::new();
        let color = builder.add_transient_color("color", TextureFormat::Rgba8Unorm);
        let _ = builder.add_subpass("s0").writes_color(color);
        let graph = builder.build().unwrap();

        let mask = graph.resolve_active(&[SubpassIndex(0)]).unwrap();
        assert!(mask.is_active(SubpassIndex(0)));
    }

    #[test]
    fn send_sync_assertion_compiles() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RenderGraph>();
    }

    #[test]
    fn attachment_id_is_opaque_but_comparable() {
        assert_eq!(attachment(2), attachment(2));
        assert_ne!(attachment(1), attachment(2));
    }

    #[test]
    fn build_duplicate_color_write_is_rejected() {
        let mut builder = RenderGraphBuilder::new();
        let color = builder.add_transient_color("color", TextureFormat::Rgba8Unorm);
        let _ = builder
            .add_subpass("dup")
            .writes_color(color)
            .writes_color(color);
        let err = builder.build().unwrap_err();
        assert!(matches!(
            err,
            RenderGraphError::DuplicateAttachmentWrite {
                subpass: SubpassIndex(0),
                attachment,
            } if attachment == color
        ));
    }

    #[test]
    fn descriptor_views_expose_subpass_and_dependency_data() {
        let mut builder = RenderGraphBuilder::new();
        let gbuffer = builder.add_transient_color("gbuffer", TextureFormat::Rgba8Unorm);
        let out = builder.add_persistent_color("out", TextureFormat::Rgba8Unorm);
        let _ = builder.add_subpass("s0").writes_color(gbuffer);
        let _ = builder.add_subpass("s1").reads(gbuffer).writes_color(out);
        let graph = builder.build().unwrap();
        let views = graph.descriptor_views();

        assert_eq!(views.subpasses.len(), 2);
        assert_eq!(views.subpass_dependencies.len(), 1);
        assert_eq!(views.subpass_layouts.len(), 2);
        assert_eq!(views.subpasses[0].color_attachment_indices, &[0]);
        assert_eq!(views.subpasses[1].color_attachment_indices, &[1]);
        assert_eq!(views.subpasses[1].input_attachments.len(), 1);
    }

    #[test]
    fn make_subpass_target_matches_graph_metadata() {
        let mut builder = RenderGraphBuilder::new();
        let gbuffer = builder.add_transient_color("gbuffer", TextureFormat::Rgba8Unorm);
        let out = builder.add_persistent_color("out", TextureFormat::Rgba8Unorm);
        let _ = builder.add_subpass("s0").writes_color(gbuffer);
        let _ = builder.add_subpass("s1").reads(gbuffer).writes_color(out);
        let graph = builder.build().unwrap();

        let target = graph.make_subpass_target(
            1,
            &[
                Some(TextureFormat::Rgba8Unorm),
                Some(TextureFormat::Rgba8Unorm),
            ],
            None,
        );
        assert_eq!(target.index, 1);
        assert_eq!(target.subpass_descs.len(), 2);
        assert_eq!(target.subpass_descs[0].color_attachment_indices, vec![0]);
        assert_eq!(target.subpass_descs[1].color_attachment_indices, vec![1]);
        assert_eq!(target.subpass_descs[1].input_attachment_indices, vec![0]);
        assert_eq!(target.dependencies, graph.dependencies);
    }

    #[test]
    fn labels_are_preserved_in_render_graph() {
        let mut builder = RenderGraphBuilder::new();
        let color = builder.add_transient_color("color-label", TextureFormat::Rgba8Unorm);
        let _ = builder.add_subpass("subpass-label").writes_color(color);
        let graph = builder.build().unwrap();

        assert_eq!(graph.attachment_label(color), Some("color-label"));
        assert_eq!(graph.subpass_label(SubpassIndex(0)), Some("subpass-label"));
    }
}
