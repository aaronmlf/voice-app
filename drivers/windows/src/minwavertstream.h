/* Copyright (c) Microsoft Corporation. All rights reserved.
 * Voice App replacement stream implementation. SPDX-License-Identifier: MS-PL */
#pragma once
#include "voicebridge.h"

class CMiniportWaveRT;
EXT_CALLBACK TimerNotifyRT;
class CMiniportWaveRTStream : public IDrmAudioStream,
    public IMiniportWaveRTStreamNotification,
    public IMiniportWaveRTInputStream,
    public IMiniportWaveRTOutputStream, public CUnknown {
public:
    DECLARE_STD_UNKNOWN();
    explicit CMiniportWaveRTStream(PUNKNOWN outer) : CUnknown(outer) { KeInitializeMutex(&m_Control, 0); }
    ~CMiniportWaveRTStream();
    IMP_IMiniportWaveRTStream;
    IMP_IMiniportWaveRTStreamNotification;
    IMP_IMiniportWaveRTInputStream;
    IMP_IMiniportWaveRTOutputStream;
    IMP_IDrmAudioStream;
    NTSTATUS Init(CMiniportWaveRT*, PPORTWAVERTSTREAM, ULONG, BOOLEAN, PKSDATAFORMAT, GUID);
    GUID GetSignalProcessingMode() { return m_Mode; }
    friend class CMiniportWaveRT;
    friend EXT_CALLBACK TimerNotifyRT;
private:
    static constexpr ULONG MaxBufferBytes = 65536;
    static constexpr ULONG MaxEvents = 8;
    KMUTEX m_Control{}; // PASSIVE setup only; never acquired by processing.
    CMiniportWaveRT* m_Miniport = nullptr;
    PADAPTERCOMMON m_Adapter = nullptr;
    PPORTWAVERTSTREAM m_Port = nullptr; // PortCls owns enclosing stream lifetime.
    VoiceBridge* m_Bridge = nullptr; // adapter outlives miniport/stream references
    ULONG m_Pin = 0;
    BOOLEAN m_Capture = FALSE, m_Registered = FALSE;
    PMDL m_Mdl = nullptr;
    uint32_t* m_Buffer = nullptr;
    VoiceAtomic64 m_BufferBytes, m_Notifications;
    PKEVENT m_Events[MaxEvents]{};
    PEX_TIMER m_Timer = nullptr;
    VoiceFrameClock m_Clock;
    uint64_t m_Frequency = 1;
    VoiceAtomic64 m_State, m_Frames, m_Qpc, m_SnapshotSequence, m_CommittedFrames, m_EndFrame;
    VoiceAtomic64 m_LastReadPacket;
    VoiceAtomic64 m_Arming, m_ProcessOwner;
    VoiceAtomic64 m_Epoch, m_PacketEpoch, m_WriteOwner;
    VoiceAtomic64 m_DrmAllowed;
    GUID m_Mode{};
    ULONG m_ulContentId = 0; // miniport DRM bookkeeping only
    void Quiesce();
    NTSTATUS Arm();
    NTSTATUS Allocate(ULONG, ULONG, PMDL*, ULONG*, ULONG*, MEMORY_CACHING_TYPE*);
    void Free(PMDL);
    bool Snapshot(uint64_t&, uint64_t&);
    void Process(uint64_t);
};
