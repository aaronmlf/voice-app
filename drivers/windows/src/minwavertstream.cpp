/* Copyright (c) Microsoft Corporation. All rights reserved.
 * Voice App replacement stream implementation. SPDX-License-Identifier: MS-PL */
#include "definitions.h"
#include "endpoints.h"
#include "minwavert.h"
#include "minwavertstream.h"

// Serializes only PASSIVE control requests. The DPC never touches this mutex.
class VoiceControlGuard {
    KMUTEX* mutex_;
public:
    explicit VoiceControlGuard(KMUTEX& mutex) : mutex_(&mutex) {
        NT_ASSERT(KeGetCurrentIrql() == PASSIVE_LEVEL);
        KeWaitForSingleObject(mutex_, Executive, KernelMode, FALSE, nullptr);
    }
    ~VoiceControlGuard() { KeReleaseMutex(mutex_, FALSE); }
};

#pragma code_seg("PAGE")
CMiniportWaveRTStream::~CMiniportWaveRTStream() {
    PAGED_CODE();
    Quiesce(); // MUST precede event, MDL, miniport or bridge destruction.
    if (!m_Capture && m_Bridge) m_Bridge->StopProducer();
    Free(m_Mdl);
    if (m_Miniport) {
        if (m_Registered) m_Miniport->StreamClosed(m_Pin, this);
        m_Miniport->Release();
    }
    if (m_Adapter) m_Adapter->Release();
}

NTSTATUS CMiniportWaveRTStream::Init(CMiniportWaveRT* miniport,
    PPORTWAVERTSTREAM port, ULONG pin, BOOLEAN capture, PKSDATAFORMAT format, GUID mode) {
    PAGED_CODE();
    if (!miniport || !port || !format) return STATUS_INVALID_PARAMETER;
    const auto wave = GetWaveFormatEx(format);
    if (!wave || wave->nSamplesPerSec != 48000 || wave->nChannels != 2 ||
        wave->wBitsPerSample != 16 || wave->nBlockAlign != 4 || wave->nAvgBytesPerSec != 192000)
        return STATUS_NO_MATCH;
    m_Miniport = miniport;
    miniport->AddRef();
    m_Port = port;
    m_Pin = pin;
    m_Capture = capture;
    m_Mode = mode;
    m_Adapter = miniport->GetAdapterCommObj();
    m_Adapter->AddRef();
    m_Bridge = m_Adapter->GetVoiceBridge();
    m_State.Store(KSSTATE_STOP);
    m_Epoch.Store(1);
    m_DrmAllowed.Store(1);
    m_EndFrame.Store(UINT64_MAX);
    m_LastReadPacket.Store(UINT64_MAX);
    LARGE_INTEGER frequency;
    KeQueryPerformanceCounter(&frequency);
    if (frequency.QuadPart <= 0 || frequency.QuadPart > 1000000000LL || !m_Bridge)
        return STATUS_DEVICE_CONFIGURATION_ERROR;
    m_Frequency = static_cast<uint64_t>(frequency.QuadPart);
    auto status = miniport->StreamCreated(pin, this);
    m_Registered = NT_SUCCESS(status);
    return status;
}

void CMiniportWaveRTStream::Quiesce() {
    PAGED_CODE();
    m_Arming.Store(0);
    if (m_Timer) {
        // Wait=TRUE drains every callback and prevents self-rearming while deleting.
        // Caller holds control ownership; never invoked by a process callback.
        ExDeleteTimer(m_Timer, TRUE, TRUE, nullptr);
        m_Timer = nullptr;
    }
}

NTSTATUS CMiniportWaveRTStream::Arm() {
    PAGED_CODE();
    if (m_State.Load() != KSSTATE_RUN) return STATUS_SUCCESS;
    m_Timer = ExAllocateTimer(TimerNotifyRT, this, EX_TIMER_HIGH_RESOLUTION);
    if (!m_Timer) return STATUS_INSUFFICIENT_RESOURCES;
    m_Clock.Reset(static_cast<uint64_t>(KeQueryPerformanceCounter(nullptr).QuadPart), m_Frequency);
    m_Arming.Store(1);
    ExSetTimer(m_Timer, -10000LL, 0, nullptr); // one-shot, initially 1 ms
    return STATUS_SUCCESS;
}

STDMETHODIMP_(NTSTATUS) CMiniportWaveRTStream::NonDelegatingQueryInterface(REFIID iid, PVOID* object) {
    PAGED_CODE();
    if (!object) return STATUS_INVALID_PARAMETER;
    *object = nullptr;
    if (IsEqualGUIDAligned(iid, IID_IUnknown) || IsEqualGUIDAligned(iid, IID_IMiniportWaveRTStream))
        *object = static_cast<PMINIPORTWAVERTSTREAM>(this);
    else if (IsEqualGUIDAligned(iid, IID_IMiniportWaveRTStreamNotification))
        *object = static_cast<PMINIPORTWAVERTSTREAMNOTIFICATION>(this);
    else if (m_Capture && IsEqualGUIDAligned(iid, IID_IMiniportWaveRTInputStream))
        *object = static_cast<PMINIPORTWAVERTINPUTSTREAM>(this);
    else if (!m_Capture && IsEqualGUIDAligned(iid, IID_IMiniportWaveRTOutputStream))
        *object = static_cast<PMINIPORTWAVERTOUTPUTSTREAM>(this);
    else if (IsEqualGUIDAligned(iid, IID_IDrmAudioStream))
        *object = static_cast<IDrmAudioStream*>(this);
    if (!*object) return STATUS_INVALID_PARAMETER;
    reinterpret_cast<PUNKNOWN>(*object)->AddRef();
    return STATUS_SUCCESS;
}

NTSTATUS CMiniportWaveRTStream::Allocate(ULONG requested, ULONG notifications, PMDL* mdl,
    ULONG* actual, ULONG* offset, MEMORY_CACHING_TYPE* cache) {
    PAGED_CODE();
    if (!mdl || !actual || !offset || !cache) return STATUS_INVALID_PARAMETER;
    *mdl = nullptr; *actual = 0; *offset = 0;
    if (m_Mdl || m_State.Load() == KSSTATE_RUN) return STATUS_INVALID_DEVICE_STATE;
    if (!requested || requested > MaxBufferBytes || requested % 4 || notifications > 2 ||
        (notifications && (requested % (notifications * 4)))) return STATUS_INVALID_PARAMETER;
    PHYSICAL_ADDRESS high;
    high.QuadPart = MAXLONGLONG;
    auto allocated = m_Port->AllocatePagesForMdl(high, requested);
    if (!allocated) return STATUS_INSUFFICIENT_RESOURCES;
    auto buffer = static_cast<uint32_t*>(m_Port->MapAllocatedPages(allocated, MmCached));
    if (!buffer) { m_Port->FreePagesFromMdl(allocated); return STATUS_INSUFFICIENT_RESOURCES; }
    RtlZeroMemory(buffer, requested);
    m_Mdl = allocated; m_Buffer = buffer; m_BufferBytes = requested; m_Notifications = notifications;
    *mdl = allocated; *actual = requested; *offset = 0; *cache = MmCached;
    return STATUS_SUCCESS;
}

NTSTATUS CMiniportWaveRTStream::AllocateBufferWithNotification(ULONG count, ULONG size,
    PMDL* mdl, ULONG* actual, ULONG* offset, MEMORY_CACHING_TYPE* cache) {
    PAGED_CODE(); VoiceControlGuard guard(m_Control);
    if (count != 1 && count != 2) return STATUS_INVALID_PARAMETER;
    return Allocate(size, count, mdl, actual, offset, cache);
}
NTSTATUS CMiniportWaveRTStream::AllocateAudioBuffer(ULONG size, PMDL* mdl,
    ULONG* actual, ULONG* offset, MEMORY_CACHING_TYPE* cache) {
    PAGED_CODE(); VoiceControlGuard guard(m_Control);
    // Event-driven operation is the supported transport. Polling allocates safely,
    // but no uncommitted render bytes are ever transmitted.
    return Allocate(size, 0, mdl, actual, offset, cache);
}
void CMiniportWaveRTStream::Free(PMDL mdl) {
    PAGED_CODE();
    if (!mdl || mdl != m_Mdl) return;
    Quiesce();
    if (m_Capture) m_Bridge->DropConsumer();
    m_State.Store(KSSTATE_STOP);
    if (!m_Capture && m_Bridge) m_Bridge->StopProducer();
    RtlZeroMemory(m_Buffer, m_BufferBytes);
    m_Port->UnmapAllocatedPages(m_Buffer, mdl);
    m_Port->FreePagesFromMdl(mdl);
    m_Mdl = nullptr; m_Buffer = nullptr; m_BufferBytes = 0; m_Notifications = 0;
}
void CMiniportWaveRTStream::FreeBufferWithNotification(PMDL mdl, ULONG size) {
    PAGED_CODE(); UNREFERENCED_PARAMETER(size); VoiceControlGuard guard(m_Control); Free(mdl);
}
void CMiniportWaveRTStream::FreeAudioBuffer(PMDL mdl, ULONG size) {
    PAGED_CODE(); UNREFERENCED_PARAMETER(size); VoiceControlGuard guard(m_Control); Free(mdl);
}

NTSTATUS CMiniportWaveRTStream::RegisterNotificationEvent(PKEVENT event) {
    PAGED_CODE(); VoiceControlGuard guard(m_Control);
    if (!event) return STATUS_INVALID_PARAMETER;
    ULONG empty = MaxEvents;
    for (ULONG i = 0; i < MaxEvents; ++i) {
        if (m_Events[i] == event) return STATUS_OBJECT_NAME_COLLISION;
        if (!m_Events[i]) empty = i;
    }
    if (empty == MaxEvents) return STATUS_INSUFFICIENT_RESOURCES;
    Quiesce(); m_Events[empty] = event;
    auto status = Arm();
    if (!NT_SUCCESS(status)) {
        m_Events[empty] = nullptr; m_State.Store(KSSTATE_STOP);
        if (!m_Capture) m_Bridge->StopProducer();
    }
    return status;
}
NTSTATUS CMiniportWaveRTStream::UnregisterNotificationEvent(PKEVENT event) {
    PAGED_CODE(); VoiceControlGuard guard(m_Control);
    for (ULONG i = 0; i < MaxEvents; ++i) if (m_Events[i] == event && event) {
        Quiesce(); m_Events[i] = nullptr; // drain before caller may free event
        auto status = Arm();
        if (!NT_SUCCESS(status)) {
            m_State.Store(KSSTATE_STOP); if (!m_Capture) m_Bridge->StopProducer();
        }
        return status;
    }
    return STATUS_NOT_FOUND;
}

NTSTATUS CMiniportWaveRTStream::SetState(KSSTATE state) {
    PAGED_CODE(); VoiceControlGuard guard(m_Control);
    if (state < KSSTATE_STOP || state > KSSTATE_RUN) return STATUS_INVALID_PARAMETER;
    const auto previous = m_State.Load();
    if (previous == static_cast<uint64_t>(state)) return STATUS_SUCCESS;
    if (state == KSSTATE_RUN && (!m_Buffer || !m_Notifications)) return STATUS_NOT_SUPPORTED;
    if (state == KSSTATE_RUN && !m_DrmAllowed.Load()) return STATUS_ACCESS_DENIED;
    m_State.Store(state);
    if (!m_Capture && state != KSSTATE_RUN) m_Bridge->StopProducer();
    Quiesce();
    if (m_Capture) m_Bridge->DropConsumer();
    if (state != KSSTATE_RUN) {
        m_Epoch.Store(m_Epoch.Load() + 1);
        m_PacketEpoch.Store(0); m_CommittedFrames.Store(0); m_EndFrame.Store(UINT64_MAX);
        if (m_Buffer) RtlZeroMemory(m_Buffer, m_BufferBytes);
    }
    if (state == KSSTATE_STOP) {
        m_DrmAllowed.Store(1); m_ulContentId = 0;
        m_SnapshotSequence.Store(m_SnapshotSequence.Load() + 1);
        m_Frames.Store(0); m_Qpc.Store(0);
        m_SnapshotSequence.Store(m_SnapshotSequence.Load() + 1);
        m_CommittedFrames.Store(0); m_EndFrame.Store(UINT64_MAX); m_LastReadPacket.Store(UINT64_MAX);
        if (m_Buffer) RtlZeroMemory(m_Buffer, m_BufferBytes);
    }
    if (state == KSSTATE_RUN) {
        if (!m_Capture) m_Bridge->StartProducer();
        auto status = Arm();
        if (!NT_SUCCESS(status)) {
            m_State.Store(KSSTATE_STOP); if (!m_Capture) m_Bridge->StopProducer();
        }
        return status;
    }
    return STATUS_SUCCESS;
}
NTSTATUS CMiniportWaveRTStream::SetFormat(KSDATAFORMAT* format) {
    PAGED_CODE(); UNREFERENCED_PARAMETER(format); return STATUS_NOT_SUPPORTED;
}
NTSTATUS CMiniportWaveRTStream::GetClockRegister(PKSRTAUDIO_HWREGISTER reg) {
    PAGED_CODE(); UNREFERENCED_PARAMETER(reg); return STATUS_NOT_SUPPORTED;
}
NTSTATUS CMiniportWaveRTStream::GetPositionRegister(PKSRTAUDIO_HWREGISTER reg) {
    PAGED_CODE(); UNREFERENCED_PARAMETER(reg); return STATUS_NOT_SUPPORTED;
}
void CMiniportWaveRTStream::GetHWLatency(PKSRTAUDIO_HWLATENCY latency) {
    PAGED_CODE(); RtlZeroMemory(latency, sizeof(*latency));
}
STDMETHODIMP_(NTSTATUS) CMiniportWaveRTStream::SetContentId(ULONG id, PCDRMRIGHTS rights) {
    PAGED_CODE(); VoiceControlGuard guard(m_Control);
    if (!rights) return STATUS_INVALID_PARAMETER;
    // Normal unprotected shared audio must be accepted. No downstream protected
    // path exists: any restriction silences this stream and is rejected with the
    // documented unsupported-rights status, without changing its content ID.
    if (rights->CopyProtect || rights->DigitalOutputDisable || rights->Reserved) {
        m_DrmAllowed.Store(0);
        if (!m_Capture) m_Bridge->StopProducer();
        Quiesce();
        if (m_Buffer) RtlZeroMemory(m_Buffer, m_BufferBytes);
        if (!NT_SUCCESS(Arm())) m_State.Store(KSSTATE_STOP);
        return STATUS_NOT_IMPLEMENTED;
    }
    if (!m_DrmAllowed.Load()) return STATUS_INVALID_DEVICE_STATE;
    m_ulContentId = id;
    return STATUS_SUCCESS;
}

#pragma code_seg()
bool CMiniportWaveRTStream::Snapshot(uint64_t& frames, uint64_t& qpc) {
    // Bounded retry; never wait for a preempted writer.
    for (unsigned attempt = 0; attempt < 3; ++attempt) {
        const auto sequence = m_SnapshotSequence.Load();
        if (sequence & 1) continue;
        frames = m_Frames.Load(); qpc = m_Qpc.Load();
        if (m_SnapshotSequence.Load() == sequence) return true;
    }
    return false;
}
NTSTATUS CMiniportWaveRTStream::GetPosition(KSAUDIO_POSITION* position) {
    uint64_t frames, qpc;
    if (!position || !Snapshot(frames, qpc)) return STATUS_DEVICE_NOT_READY;
    UNREFERENCED_PARAMETER(qpc);
    const auto bytes = m_BufferBytes.Load();
    position->PlayOffset = position->WriteOffset = bytes ? (frames % (bytes / 4)) * 4 : 0;
    return STATUS_SUCCESS;
}
NTSTATUS CMiniportWaveRTStream::GetReadPacket(ULONG* number, DWORD* flags, ULONG64* timestamp, BOOL* more) {
    uint64_t frames, qpc;
    if (!number || !flags || !timestamp || !more) return STATUS_INVALID_PARAMETER;
    const auto bytes = m_BufferBytes.Load(), notifications = m_Notifications.Load();
    if (!m_Capture || !notifications || !bytes || !Snapshot(frames, qpc)) return STATUS_DEVICE_NOT_READY;
    const auto packetFrames = bytes / 4 / notifications;
    const auto complete = frames / packetFrames;
    if (!complete || complete - 1 == m_LastReadPacket.Load()) return STATUS_DEVICE_NOT_READY;
    *number = static_cast<ULONG>(complete - 1); *flags = 0; *more = FALSE;
    const auto ageFrames = frames - (complete - 1) * packetFrames;
    const auto ageQpc = ageFrames * m_Frequency / 48000;
    *timestamp = qpc >= ageQpc ? qpc - ageQpc : 0;
    m_LastReadPacket.Store(complete - 1);
    return STATUS_SUCCESS;
}
NTSTATUS CMiniportWaveRTStream::SetWritePacket(ULONG number, DWORD flags, ULONG eosLength) {
    const auto bytes = m_BufferBytes.Load(), notifications = m_Notifications.Load();
    const auto epoch = m_Epoch.Load();
    if (m_Capture || !notifications || !bytes) return STATUS_NOT_SUPPORTED;
    if (m_State.Load() < KSSTATE_PAUSE) return STATUS_INVALID_DEVICE_STATE;
    if (!m_WriteOwner.TryEnter()) return STATUS_DEVICE_BUSY;
    struct ReleaseOwner { VoiceAtomic64& owner; ~ReleaseOwner() { owner.Store(0); } } release{m_WriteOwner};
    if (flags & ~KSSTREAM_HEADER_OPTIONSF_ENDOFSTREAM) return STATUS_INVALID_PARAMETER;
    if (m_PacketEpoch.Load() == epoch && m_EndFrame.Load() != UINT64_MAX)
        return STATUS_INVALID_DEVICE_STATE;
    const auto packetFrames = bytes / 4 / notifications;
    const auto current = m_Frames.Load() / packetFrames;
    const auto expected = current + (m_State.Load() == KSSTATE_RUN ? 1 : 0);
    const auto delta = static_cast<LONG>(number - static_cast<ULONG>(expected));
    if (delta < 0) return STATUS_DATA_LATE_ERROR;
    if (delta > 0) return STATUS_DATA_OVERRUN;
    auto end = (expected + 1) * packetFrames;
    if (m_Epoch.Load() != epoch) return STATUS_INVALID_DEVICE_STATE;
    m_PacketEpoch.Store(0); // invalidate mailbox until its entire update is visible
    if (flags & KSSTREAM_HEADER_OPTIONSF_ENDOFSTREAM) {
        if (eosLength > packetFrames * 4 || eosLength % 4) return STATUS_INVALID_PARAMETER;
        end = expected * packetFrames + eosLength / 4;
        m_EndFrame.Store(end);
    } else m_EndFrame.Store(UINT64_MAX);
    m_CommittedFrames.Store(end);
    m_PacketEpoch.Store(epoch); // old in-flight writes cannot authorize a new RUN
    return STATUS_SUCCESS;
}
NTSTATUS CMiniportWaveRTStream::GetPacketCount(ULONG* count) {
    const auto bytes = m_BufferBytes.Load(), notifications = m_Notifications.Load();
    if (!count || !notifications || !bytes) return STATUS_NOT_SUPPORTED;
    *count = static_cast<ULONG>(m_Frames.Load() / (bytes / 4 / notifications));
    return STATUS_SUCCESS;
}
NTSTATUS CMiniportWaveRTStream::GetOutputStreamPresentationPosition(KSAUDIO_PRESENTATION_POSITION* position) {
    uint64_t frames, qpc;
    if (!position || !Snapshot(frames, qpc)) return STATUS_DEVICE_NOT_READY;
    position->u64PositionInBlocks = frames; position->u64QPCPosition = qpc;
    return STATUS_SUCCESS;
}

void CMiniportWaveRTStream::Process(uint64_t now) {
    if (m_State.Load() != KSSTATE_RUN || !m_Buffer || !m_BufferBytes) return;
    bool discontinuity;
    const auto advance = m_Clock.Advance(now, discontinuity);
    if (!advance) return;
    const uint64_t before = m_Frames.Load();
    const ULONG capacity = static_cast<ULONG>(m_BufferBytes.Load() / 4);
    const auto bounded = advance < capacity ? static_cast<ULONG>(advance) : capacity;
    // At most one cyclic-buffer worth of data is touched even after a long stall.
    const auto skip = advance - bounded;
    if (discontinuity || skip) {
        if (m_Capture) m_Bridge->DropConsumer();
        else {
            m_Bridge->StopProducer();
            if (m_DrmAllowed.Load()) m_Bridge->StartProducer();
        }
    }
    uint64_t cursor = before + skip;
    ULONG remaining = bounded;
    const auto epoch = m_PacketEpoch.Load();
    auto committed = m_CommittedFrames.Load();
    if (!epoch || epoch != m_Epoch.Load() || epoch != m_PacketEpoch.Load()) committed = 0;
    while (remaining) {
        const ULONG offset = static_cast<ULONG>(cursor % capacity);
        ULONG count = remaining < capacity - offset ? remaining : capacity - offset;
        if (count > VoiceBridge::Capacity) count = VoiceBridge::Capacity;
        auto destination = m_Buffer + offset;
        if (m_Capture) {
            if (discontinuity || skip || !m_DrmAllowed.Load()) RtlZeroMemory(destination, count * 4);
            else m_Bridge->Pop(destination, count);
        } else {
            ULONG valid = 0;
            if (!discontinuity && !skip && m_DrmAllowed.Load() && committed > cursor) {
                const auto available = committed - cursor;
                valid = available < count ? static_cast<ULONG>(available) : count;
            }
            if (valid) m_Bridge->Push(destination, valid);
            // Consumed or uncommitted render pages cannot be replayed next cycle.
            RtlZeroMemory(destination, count * 4);
        }
        cursor += count; remaining -= count;
    }
    const auto after = before + advance;
    m_SnapshotSequence.Store(m_SnapshotSequence.Load() + 1);
    m_Frames.Store(after); m_Qpc.Store(now);
    m_SnapshotSequence.Store(m_SnapshotSequence.Load() + 1);
    const auto packetFrames = capacity / m_Notifications;
    if (after / packetFrames != before / packetFrames) {
        for (ULONG i = 0; i < MaxEvents; ++i)
            if (m_Events[i]) KeSetEvent(m_Events[i], IO_NO_INCREMENT, FALSE);
    }
}

void TimerNotifyRT(PEX_TIMER timer, PVOID context) {
    auto stream = static_cast<CMiniportWaveRTStream*>(context);
    // One bounded CAS, no spinning or waiting; a redundant callback leaves rearm
    // to the owner. Timer periodicity is disabled as a second independent guard.
    if (!stream->m_ProcessOwner.TryEnter()) return;
    if (stream->m_Arming.Load())
        stream->Process(static_cast<uint64_t>(KeQueryPerformanceCounter(nullptr).QuadPart));
    stream->m_ProcessOwner.Store(0);
    // A periodic EX_TIMER may run on two CPUs simultaneously. Re-arm a ONE-SHOT
    // only after every state mutation; nothing touches stream state after this.
    // ExDeleteTimer(Wait=TRUE) disables rearming and drains callbacks at teardown.
    if (stream->m_Arming.Load()) ExSetTimer(timer, -10000LL, 0, nullptr);
}
