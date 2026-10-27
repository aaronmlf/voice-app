//! Fixed driver identity checks; friendly labels are never an ownership proof.

use voice_core::DeviceKind;

const ADAPTER_ROOT: &str = r"\\?\root#voiceappvirtualaudio";
const ADAPTER_PREFIX: &str = r"\\?\root#voiceappvirtualaudio#";

/// Accept a raw adapter interface or the exact MMDevice wrapper observed on the
/// tested Windows target. MMDevice IDs are otherwise opaque: unknown envelopes
/// fail closed. Preserve the interface/reference verbatim for native PnP checks;
/// this never normalizes the endpoint IDs used for selection or routing.
pub(crate) fn adapter_interface_path(adapter_path: &str) -> Option<&str> {
    if adapter_path.len() > 4096 || adapter_path.contains('\0') {
        return None;
    }
    let path = adapter_path.strip_prefix("{2}.").unwrap_or(adapter_path);
    let interface = path.strip_prefix(r"\\?\")?;
    (!interface.is_empty()).then_some(path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnedRole {
    Render,
    Capture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OwnedIdentity {
    pub(crate) instance: String,
    pub(crate) role: Option<OwnedRole>,
}

#[derive(Clone, Debug)]
pub(crate) struct OwnedEndpoint {
    pub(crate) id: String,
    pub(crate) identity: OwnedIdentity,
    pub(crate) kind: DeviceKind,
}

/// A single complete pair from the same PnP instance is required. Even an extra
/// unrecognized owned reference makes the selection ambiguous and unavailable.
pub(crate) fn verified_pair(
    endpoints: &[OwnedEndpoint],
) -> Option<(&OwnedEndpoint, &OwnedEndpoint)> {
    if endpoints.len() != 2 {
        return None;
    }
    let render = endpoints.iter().find(|endpoint| {
        endpoint.kind == DeviceKind::Output && endpoint.identity.role == Some(OwnedRole::Render)
    })?;
    let capture = endpoints.iter().find(|endpoint| {
        endpoint.kind == DeviceKind::Input && endpoint.identity.role == Some(OwnedRole::Capture)
    })?;
    (render.identity.instance == capture.identity.instance && render.id != capture.id)
        .then_some((render, capture))
}

/// Ownership is checked before parsing a publishable role. A damaged owned path
/// must stay excluded from physical routes, even when its GUID/reference is invalid.
pub(crate) fn is_owned_adapter_path(adapter_path: &str) -> bool {
    let Some(adapter_path) = adapter_interface_path(adapter_path) else {
        return false;
    };
    let path = adapter_path.to_ascii_lowercase();
    path.strip_prefix(ADAPTER_ROOT)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['#', '\\', '/']))
}

/// Adapter device-interface IDs come from the endpoint's connected topology,
/// not from an editable endpoint label. Keep an unknown owned reference excluded
/// from physical/default routes while refusing to publish through it.
pub(crate) fn owned_identity(adapter_path: &str) -> Option<OwnedIdentity> {
    let adapter_path = adapter_interface_path(adapter_path)?;
    let path = adapter_path.to_ascii_lowercase();
    let rest = path.strip_prefix(ADAPTER_PREFIX)?;
    let (instance, interface) = rest.split_once('#')?;
    if instance.is_empty()
        || !instance
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'&' | b'_' | b'-'))
    {
        return None;
    }
    let (category, reference) = interface.split_once('\\')?;
    if !guid(category) || reference.contains(['\\', '#', '/']) {
        return None;
    }
    let role = match reference {
        "wavevoiceapprender" | "topologyvoiceapprender" => Some(OwnedRole::Render),
        "wavevoiceappcapture" | "topologyvoiceappcapture" => Some(OwnedRole::Capture),
        _ => None,
    };
    Some(OwnedIdentity {
        instance: instance.to_owned(),
        role,
    })
}

fn guid(value: &str) -> bool {
    let Some(value) = value.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
        return false;
    };
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    const PREFIX: &str =
        r"\\?\ROOT#VoiceAppVirtualAudio#0000#{6994AD04-93EF-11D0-A3CC-00A0C9223196}\";

    #[test]
    fn observed_adapter_wrapper_preserves_hardware_paths_and_reference_strings() {
        for path in [
            r"\\?\HDAUDIO#FUNC_01&VEN_1234&DEV_5678#4&synthetic&0&0001#{6994AD04-93EF-11D0-A3CC-00A0C9223196}\eSpeakerTopo",
            r"\\?\ROOT#MEDIA#0000#{6994AD04-93EF-11D0-A3CC-00A0C9223196}\VBAudioVACWDM2022_Out1_Topo_E51A78A01FB100001",
            r"\\?\ROOT#VoiceAppVirtualAudio#0000#{6994AD04-93EF-11D0-A3CC-00A0C9223196}\TopologyVoiceAppCapture",
        ] {
            assert_eq!(adapter_interface_path(path), Some(path));
            assert_eq!(adapter_interface_path(&format!("{{2}}.{path}")), Some(path));
        }
        let path = format!("{PREFIX}TopologyVoiceAppCapture");
        assert_eq!(
            owned_identity(&format!("{{2}}.{path}")),
            owned_identity(&path)
        );
    }

    #[test]
    fn unknown_adapter_envelopes_and_invalid_boundaries_fail_closed() {
        for path in [
            String::new(),
            r"\\?\".into(),
            "{2}.".into(),
            r"{2}.\\?\".into(),
            format!("{{2}}.{{2}}.{PREFIX}WaveVoiceAppRender"),
            format!("{{3}}.{PREFIX}WaveVoiceAppRender"),
            format!("{{0.0.0.00000000}}.{PREFIX}WaveVoiceAppRender"),
            format!(" {{2}}.{PREFIX}WaveVoiceAppRender"),
            format!("{{2}}. {PREFIX}WaveVoiceAppRender"),
            "{2}.Voice App Virtual Microphone".into(),
            format!("{PREFIX}WaveVoiceAppRender\0"),
            format!("{{2}}.{PREFIX}WaveVoiceAppRender\0"),
            format!(r"\\?\{}", "x".repeat(4093)),
            format!(r"{{2}}.\\?\{}", "x".repeat(4089)),
        ] {
            assert!(adapter_interface_path(&path).is_none(), "{path:?}");
            assert!(owned_identity(&path).is_none(), "{path:?}");
        }
        let longest = format!(r"\\?\{}", "x".repeat(4092));
        assert_eq!(adapter_interface_path(&longest), Some(longest.as_str()));
        assert!(adapter_interface_path(&format!("{{2}}.{longest}")).is_none());
    }

    #[test]
    fn exact_hardware_and_reference_match_without_friendly_names() {
        for reference in ["WaveVoiceAppRender", "TopologyVoiceAppRender"] {
            assert_eq!(
                owned_identity(&format!("{PREFIX}{reference}")),
                Some(OwnedIdentity {
                    instance: "0000".into(),
                    role: Some(OwnedRole::Render)
                })
            );
        }
        assert_eq!(
            owned_identity(&format!("{PREFIX}TopologyVoiceAppCapture"))
                .unwrap()
                .role,
            Some(OwnedRole::Capture)
        );
    }

    #[test]
    fn similar_names_other_hardware_and_malformed_paths_do_not_match() {
        for path in [
            "Voice App Virtual Microphone".to_owned(),
            format!("{PREFIX}WaveVoiceAppRender\\unexpected"),
            format!("{}WaveVoiceAppRender", PREFIX.replace("#0000#", "##")),
            format!(
                "{}WaveVoiceAppRender",
                PREFIX.replace("VirtualAudio#", "VirtualAudioFake#")
            ),
            format!(
                "{}WaveVoiceAppRender",
                PREFIX.replace("6994AD04", "zzzzzzzz")
            ),
        ] {
            assert!(owned_identity(&path).is_none(), "{path}");
        }
    }

    #[test]
    fn unknown_owned_reference_is_still_owned_but_not_publishable() {
        assert_eq!(
            owned_identity(&format!("{PREFIX}UnexpectedReference")),
            Some(OwnedIdentity {
                instance: "0000".into(),
                role: None,
            })
        );
    }

    fn endpoint(id: &str, role: OwnedRole, instance: &str) -> OwnedEndpoint {
        OwnedEndpoint {
            id: id.into(),
            kind: match role {
                OwnedRole::Render => DeviceKind::Output,
                OwnedRole::Capture => DeviceKind::Input,
            },
            identity: OwnedIdentity {
                instance: instance.into(),
                role: Some(role),
            },
        }
    }

    #[test]
    fn pair_requires_both_directions_from_one_instance_and_no_duplicates() {
        let render = endpoint("render-id", OwnedRole::Render, "0000");
        let capture = endpoint("capture-id", OwnedRole::Capture, "0000");
        assert!(verified_pair(&[]).is_none());
        assert!(verified_pair(std::slice::from_ref(&render)).is_none());
        let pair = [capture.clone(), render.clone()];
        let (found_render, found_capture) = verified_pair(&pair).unwrap();
        assert_eq!(found_render.id, "render-id");
        assert_eq!(found_capture.id, "capture-id");
        assert!(verified_pair(&[render.clone(), render.clone()]).is_none());
        assert!(verified_pair(&[render.clone(), capture.clone(), capture]).is_none());
        assert!(
            verified_pair(&[
                render.clone(),
                endpoint("capture-id", OwnedRole::Capture, "0001")
            ])
            .is_none()
        );
        assert!(
            verified_pair(&[render, endpoint("render-id", OwnedRole::Capture, "0000")]).is_none()
        );
    }

    #[test]
    fn wrong_direction_and_unknown_owned_reference_cannot_be_published() {
        let render = endpoint("render-id", OwnedRole::Render, "0000");
        let mut capture = endpoint("capture-id", OwnedRole::Capture, "0000");
        capture.kind = DeviceKind::Output;
        assert!(verified_pair(&[render.clone(), capture.clone()]).is_none());
        capture.kind = DeviceKind::Input;
        capture.identity.role = None;
        assert!(verified_pair(&[render, capture]).is_none());
    }

    #[test]
    fn malformed_owned_paths_remain_owned_and_cannot_become_physical_routes() {
        for path in [
            r"\\?\root#voiceappvirtualaudio",
            r"\\?\root#voiceappvirtualaudio#",
            r"\\?\ROOT#VoiceAppVirtualAudio##{6994ad04-93ef-11d0-a3cc-00a0c9223196}\WaveVoiceAppRender",
            r"\\?\root#voiceappvirtualaudio#0000#{not-a-guid}\WaveVoiceAppRender",
            r"\\?\root#voiceappvirtualaudio#0000#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\WaveVoiceAppRender\unexpected",
        ] {
            assert!(is_owned_adapter_path(path), "{path}");
            assert!(owned_identity(path).is_none(), "{path}");
            let wrapped = format!("{{2}}.{path}");
            assert!(is_owned_adapter_path(&wrapped), "{wrapped}");
            assert!(owned_identity(&wrapped).is_none(), "{wrapped}");
        }
        let mut render = endpoint("render-id", OwnedRole::Render, "0000");
        render.identity = OwnedIdentity {
            instance: String::new(),
            role: None,
        };
        assert!(
            verified_pair(&[render, endpoint("capture-id", OwnedRole::Capture, "0000")]).is_none()
        );
    }

    #[test]
    fn foreign_hardware_stays_distinct_from_owned_hardware_without_labels() {
        for path in [
            r"\\?\root#voiceappvirtualaudiofake#0000#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\WaveVoiceAppRender",
            r"\\?\root#media#0000#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\WaveVoiceAppRender",
            "Voice App Virtual Microphone",
            "Renamed microphone",
        ] {
            assert!(!is_owned_adapter_path(path), "{path}");
        }
        assert!(is_owned_adapter_path(&format!(
            "{PREFIX}WaveVoiceAppRender"
        )));
    }
}

/// The externally installed base VB-CABLE profile observed in the official Pack45
/// Windows 10/11 INF. Identity is native PnP data, never editable endpoint names.
const VBCABLE_HARDWARE_ID: &str = "VBAudioVACWDM";
const VBCABLE_SERVICE: &str = "VBAudioVACMME";

/// Classification is deliberately broader than publishable identity: an incomplete
/// or unsupported VB-CABLE endpoint must never become a physical/default route.
pub(crate) fn is_vbcable_adapter(hardware_ids: &[String], service: &str) -> bool {
    hardware_ids
        .iter()
        .any(|id| id.eq_ignore_ascii_case(VBCABLE_HARDWARE_ID))
        || service.eq_ignore_ascii_case(VBCABLE_SERVICE)
}

pub(crate) fn vbcable_identity(
    adapter_path: &str,
    instance: &str,
    hardware_ids: &[String],
    service: &str,
) -> Option<OwnedIdentity> {
    let adapter_path = adapter_interface_path(adapter_path)?;
    if !hardware_ids
        .iter()
        .any(|id| id.eq_ignore_ascii_case(VBCABLE_HARDWARE_ID))
        || !service.eq_ignore_ascii_case(VBCABLE_SERVICE)
    {
        return None;
    }
    // This instance comes from ConfigManager for this interface, not from an
    // endpoint ID, friendly label, or assumptions about ROOT\\MEDIA numbering.
    if instance.len() > 4096 {
        return None;
    }
    let parts: Vec<_> = instance.split('\\').collect();
    if parts.len() != 3
        || parts.iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'&' | b'_' | b'-'))
        })
    {
        return None;
    }
    let path = adapter_path.to_ascii_lowercase();
    let path = path.strip_prefix(r"\\?\")?;
    let (prefix, reference) = path.rsplit_once('\\')?;
    let (device, category) = prefix.rsplit_once('#')?;
    if !guid(category)
        || device.split('#').count() != 3
        || device.split('#').any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'&' | b'_' | b'-'))
        })
        || reference.is_empty()
        || reference.contains(['\\', '#', '/'])
    {
        return None;
    }
    let role = match reference {
        "vbaudiovacwdm2022_out1_wave_e51a78a01fb100001"
        | "vbaudiovacwdm2022_out1_topo_e51a78a01fb100001" => Some(OwnedRole::Render),
        "vbaudiovacwdm2022_in1_wave_e51a78a01fb200001"
        | "vbaudiovacwdm2022_in1_topo_e51a78a01fb200001" => Some(OwnedRole::Capture),
        _ => None,
    };
    Some(OwnedIdentity {
        instance: instance.to_ascii_lowercase(),
        role,
    })
}

#[derive(Clone, Debug)]
pub(crate) struct CableEndpoint {
    pub(crate) id: String,
    pub(crate) description: String,
    pub(crate) identity: OwnedIdentity,
    pub(crate) kind: DeviceKind,
    pub(crate) supported_format: bool,
}

/// Multiple supported renders can share the one cable capture. They remain exact
/// user-selectable targets; no label or enumeration order chooses one implicitly.
/// Extra captures fail closed, including a duplicate with an unsupported format.
pub(crate) fn cable_targets(
    endpoints: &[CableEndpoint],
) -> Vec<voice_core::VirtualMicrophoneTarget> {
    let mut targets = Vec::new();
    for render in endpoints.iter().filter(|endpoint| {
        endpoint.kind == DeviceKind::Output
            && endpoint.identity.role == Some(OwnedRole::Render)
            && endpoint.supported_format
    }) {
        let mut captures = endpoints.iter().filter(|endpoint| {
            endpoint.identity.instance == render.identity.instance
                && endpoint.kind == DeviceKind::Input
                && endpoint.identity.role == Some(OwnedRole::Capture)
        });
        let Some(capture) = captures.next() else {
            continue;
        };
        if captures.next().is_some() || !capture.supported_format || capture.id == render.id {
            continue;
        }
        if targets
            .iter()
            .any(|target: &voice_core::VirtualMicrophoneTarget| target.render_device == render.id)
        {
            continue;
        }
        targets.push(voice_core::VirtualMicrophoneTarget {
            provider: "vb_cable".into(),
            render_device: render.id.clone(),
            capture_device: capture.id.clone(),
            render_description: render.description.clone(),
            capture_description: capture.description.clone(),
        });
    }
    targets.sort_by(|left, right| left.render_device.cmp(&right.render_device));
    targets
}

pub(crate) fn selected_status(
    targets: Vec<voice_core::VirtualMicrophoneTarget>,
    selected: Option<&str>,
) -> voice_core::VirtualMicrophoneStatus {
    let mut status = voice_core::VirtualMicrophoneStatus {
        targets,
        ..Default::default()
    };
    let target = match selected {
        Some(selected) => status
            .targets
            .iter()
            .find(|target| target.render_device == selected),
        None if status.targets.len() == 1 => status.targets.first(),
        None => None,
    };
    if let Some(target) = target {
        status.available = true;
        status.render_device = Some(target.render_device.clone());
        status.capture_device = Some(target.capture_device.clone());
    } else {
        status.diagnostic = Some(if selected.is_some() {
            "The saved virtual output is unavailable or unsupported; select its exact installed endpoint again. No route was substituted".into()
        } else if status.targets.is_empty() {
            "No supported VB-CABLE pair found. Install VB-CABLE separately, then set its playback and recording endpoints to 48000 Hz stereo in Windows sound settings".into()
        } else {
            "Multiple virtual outputs are available; choose one explicitly. No route is enabled automatically".into()
        });
    }
    status
}

pub(crate) fn overlaps_physical_route(
    target: &voice_core::VirtualMicrophoneTarget,
    input: &str,
    headphones: &str,
) -> bool {
    [input, headphones]
        .iter()
        .any(|id| *id == target.render_device.as_str() || *id == target.capture_device.as_str())
}

#[cfg(test)]
mod cable_tests {
    use super::*;
    const PREFIX: &str = r"\\?\ROOT#MEDIA#0000#{6994AD04-93EF-11D0-A3CC-00A0C9223196}\";
    const INSTANCE: &str = r"ROOT\MEDIA\0000";
    const RENDER: &str = "VBAudioVACWDM2022_Out1_Topo_E51A78A01FB100001";
    const CAPTURE: &str = "VBAudioVACWDM2022_In1_Topo_E51A78A01FB200001";

    fn hardware() -> Vec<String> {
        vec![VBCABLE_HARDWARE_ID.into()]
    }

    fn identity(reference: &str, instance: &str) -> OwnedIdentity {
        vbcable_identity(
            &format!("{PREFIX}{reference}"),
            instance,
            &hardware(),
            VBCABLE_SERVICE,
        )
        .unwrap()
    }

    fn endpoint(id: &str, role: OwnedRole, instance: &str) -> CableEndpoint {
        CableEndpoint {
            id: id.into(),
            description: "User renamed this endpoint".into(),
            identity: identity(
                if role == OwnedRole::Render {
                    RENDER
                } else {
                    CAPTURE
                },
                instance,
            ),
            kind: if role == OwnedRole::Render {
                DeviceKind::Output
            } else {
                DeviceKind::Input
            },
            supported_format: true,
        }
    }

    #[test]
    fn cable_uses_native_hardware_service_and_reference_without_labels() {
        for reference in [RENDER, "VBAudioVACWDM2022_Out1_Wave_E51A78A01FB100001"] {
            assert_eq!(identity(reference, INSTANCE).role, Some(OwnedRole::Render));
        }
        assert_eq!(identity(CAPTURE, INSTANCE).role, Some(OwnedRole::Capture));
        assert!(
            vbcable_identity(&format!("{PREFIX}{RENDER}"), INSTANCE, &[], VBCABLE_SERVICE)
                .is_none()
        );
        assert!(
            vbcable_identity(
                &format!("{PREFIX}{RENDER}"),
                INSTANCE,
                &hardware(),
                "OtherService"
            )
            .is_none()
        );
        assert!(!is_vbcable_adapter(
            &["VBAudioVACWDMFake".into()],
            "VBAudioVACMMEFake"
        ));
        assert!(is_vbcable_adapter(&hardware(), "UnknownService"));
        assert!(is_vbcable_adapter(&[], VBCABLE_SERVICE));
    }

    #[test]
    fn wrapped_cable_paths_still_require_native_metadata_and_safe_routing() {
        let wrapped_render = format!("{{2}}.{PREFIX}{RENDER}");
        let wrapped_capture = format!("{{2}}.{PREFIX}{CAPTURE}");
        let render_identity =
            vbcable_identity(&wrapped_render, INSTANCE, &hardware(), VBCABLE_SERVICE).unwrap();
        let capture_identity =
            vbcable_identity(&wrapped_capture, INSTANCE, &hardware(), VBCABLE_SERVICE).unwrap();
        assert_eq!(render_identity, identity(RENDER, INSTANCE));
        assert_eq!(capture_identity, identity(CAPTURE, INSTANCE));
        assert!(vbcable_identity(&wrapped_render, INSTANCE, &[], VBCABLE_SERVICE).is_none());
        assert!(vbcable_identity(&wrapped_render, INSTANCE, &hardware(), "OtherService").is_none());
        assert!(
            vbcable_identity(
                &format!("{{3}}.{PREFIX}{RENDER}"),
                INSTANCE,
                &hardware(),
                VBCABLE_SERVICE
            )
            .is_none()
        );
        let mut render = endpoint("unchanged-render-id", OwnedRole::Render, INSTANCE);
        let mut capture = endpoint("unchanged-capture-id", OwnedRole::Capture, INSTANCE);
        render.identity = render_identity;
        capture.identity = capture_identity;
        for reference in [
            "Topo",
            "FutureReference",
            "VBAudioVACWDM2022_Out2_Topo_unknown",
        ] {
            let mut unsupported = render.clone();
            unsupported.identity = vbcable_identity(
                &format!("{{2}}.{PREFIX}{reference}"),
                INSTANCE,
                &hardware(),
                VBCABLE_SERVICE,
            )
            .unwrap();
            assert!(unsupported.identity.role.is_none());
            assert!(is_vbcable_adapter(&hardware(), VBCABLE_SERVICE));
            assert!(cable_targets(&[unsupported, capture.clone()]).is_empty());
        }
        let targets = cable_targets(&[render.clone(), capture.clone()]);
        assert_eq!(targets[0].render_device, render.id);
        assert_eq!(targets[0].capture_device, capture.id);
        assert!(overlaps_physical_route(
            &targets[0],
            &capture.id,
            "headphones"
        ));
        assert!(overlaps_physical_route(
            &targets[0],
            "microphone",
            &render.id
        ));
        capture.identity.instance = r"root\media\0001".into();
        assert!(cable_targets(&[render, capture]).is_empty());
    }

    #[test]
    fn legacy_unknown_and_malformed_cable_paths_are_not_publishable() {
        for reference in [
            "Wave",
            "Topo",
            "FutureReference",
            "VBAudioVACWDM2022_Out2_Topo_unknown",
        ] {
            assert_eq!(identity(reference, INSTANCE).role, None);
        }
        for path in [
            format!("{PREFIX}{RENDER}\\extra"),
            format!("{PREFIX}{RENDER}/extra"),
            format!("{PREFIX}{RENDER}").replace("6994AD04", "not-guid"),
            "CABLE Input (VB-Audio Virtual Cable)".into(),
        ] {
            assert!(vbcable_identity(&path, INSTANCE, &hardware(), VBCABLE_SERVICE).is_none());
            assert!(
                vbcable_identity(
                    &format!("{{2}}.{path}"),
                    INSTANCE,
                    &hardware(),
                    VBCABLE_SERVICE
                )
                .is_none()
            );
            assert!(is_vbcable_adapter(&hardware(), VBCABLE_SERVICE));
        }
        for instance in [
            "",
            "ROOT\\MEDIA",
            "ROOT\\MEDIA\\",
            "ROOT\\MEDIA\\0000\\extra",
            "ROOT\\MEDIA\\0000\0",
        ] {
            assert!(
                vbcable_identity(
                    &format!("{PREFIX}{RENDER}"),
                    instance,
                    &hardware(),
                    VBCABLE_SERVICE
                )
                .is_none()
            );
        }
    }

    #[test]
    fn cable_pairs_stay_on_one_actual_pnp_adapter_and_require_one_capture() {
        let render = endpoint("render", OwnedRole::Render, INSTANCE);
        let capture = endpoint("capture", OwnedRole::Capture, INSTANCE);
        assert_eq!(cable_targets(&[render.clone(), capture.clone()]).len(), 1);
        assert!(
            cable_targets(&[
                render.clone(),
                endpoint("other-capture", OwnedRole::Capture, r"ROOT\MEDIA\0001")
            ])
            .is_empty()
        );
        assert!(cable_targets(&[render.clone(), capture.clone(), capture.clone()]).is_empty());
        let mut invalid = capture.clone();
        invalid.supported_format = false;
        assert!(cable_targets(&[render.clone(), capture.clone(), invalid]).is_empty());
        let mut invalid = capture;
        invalid.identity.role = None;
        assert!(cable_targets(&[render, invalid]).is_empty());
    }

    #[test]
    fn complete_adapters_remain_separate_manual_targets_and_wrong_flow_fails_closed() {
        let render = endpoint("render-0", OwnedRole::Render, INSTANCE);
        let capture = endpoint("capture-0", OwnedRole::Capture, INSTANCE);
        let targets = cable_targets(&[
            render.clone(),
            capture.clone(),
            endpoint("render-1", OwnedRole::Render, r"ROOT\MEDIA\0001"),
            endpoint("capture-1", OwnedRole::Capture, r"ROOT\MEDIA\0001"),
        ]);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].capture_device, "capture-0");
        assert_eq!(targets[1].capture_device, "capture-1");
        assert!(!selected_status(targets, None).available);
        let mut wrong_flow = capture;
        wrong_flow.kind = DeviceKind::Output;
        assert!(cable_targets(&[render.clone(), wrong_flow]).is_empty());
        assert!(
            cable_targets(&[render, endpoint("render-0", OwnedRole::Capture, INSTANCE)]).is_empty()
        );
    }

    #[test]
    fn unsupported_sixteen_channel_render_is_excluded_and_multiple_stereo_requires_choice() {
        let render = endpoint("render", OwnedRole::Render, INSTANCE);
        let capture = endpoint("capture", OwnedRole::Capture, INSTANCE);
        let mut surround = endpoint("surround", OwnedRole::Render, INSTANCE);
        surround.supported_format = false;
        let targets = cable_targets(&[render.clone(), capture.clone(), surround]);
        assert_eq!(targets.len(), 1);
        assert!(selected_status(targets, None).available);
        let second = endpoint("other-render", OwnedRole::Render, INSTANCE);
        let targets = cable_targets(&[render, capture, second]);
        assert_eq!(targets.len(), 2);
        let status = selected_status(targets.clone(), None);
        assert!(!status.available);
        assert!(status.render_device.is_none());
        assert_eq!(
            selected_status(targets.clone(), Some("other-render"))
                .render_device
                .as_deref(),
            Some("other-render")
        );
        let missing = selected_status(targets, Some("saved-disconnected-render"));
        assert!(!missing.available);
        assert_eq!(missing.targets.len(), 2);
    }

    #[test]
    fn renaming_does_not_affect_identity_and_virtual_endpoints_cannot_be_physical_routes() {
        let targets = cable_targets(&[
            endpoint("render", OwnedRole::Render, INSTANCE),
            endpoint("capture", OwnedRole::Capture, INSTANCE),
        ]);
        let target = &targets[0];
        assert_eq!(target.render_description, "User renamed this endpoint");
        assert!(!overlaps_physical_route(
            target,
            "usb-mic",
            "usb-headphones"
        ));
        for (input, output) in [
            ("render", "usb-headphones"),
            ("capture", "usb-headphones"),
            ("usb-mic", "render"),
            ("usb-mic", "capture"),
        ] {
            assert!(overlaps_physical_route(target, input, output));
        }
    }
}
