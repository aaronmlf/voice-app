//! Fixed driver identity checks; friendly labels are never an ownership proof.

use voice_core::DeviceKind;

const ADAPTER_ROOT: &str = r"\\?\root#voiceappvirtualaudio";
const ADAPTER_PREFIX: &str = r"\\?\root#voiceappvirtualaudio#";

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
    let path = adapter_path.to_ascii_lowercase();
    path.strip_prefix(ADAPTER_ROOT)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['#', '\\', '/']))
}

/// Adapter device-interface IDs come from the endpoint's connected topology,
/// not from an editable endpoint label. Keep an unknown owned reference excluded
/// from physical/default routes while refusing to publish through it.
pub(crate) fn owned_identity(adapter_path: &str) -> Option<OwnedIdentity> {
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
