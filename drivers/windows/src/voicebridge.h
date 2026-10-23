// Copyright (c) Voice App contributors. SPDX-License-Identifier: MIT OR Apache-2.0
#pragma once
#ifdef VOICE_APP_KERNEL
#include <ntddk.h>
using VoiceU32 = ULONG;
using VoiceU64 = ULONGLONG;
class VoiceAtomic64 {
    volatile LONG64 value_ = 0;
public:
    operator VoiceU64() const { return Load(); }
    VoiceAtomic64& operator=(VoiceU64 value) { Store(value); return *this; }
    bool TryEnter() { return InterlockedCompareExchange64(&value_, 1, 0) == 0; }
    VoiceU64 Load() const { return static_cast<VoiceU64>(InterlockedCompareExchange64(const_cast<volatile LONG64*>(&value_), 0, 0)); }
    void Store(VoiceU64 value) { InterlockedExchange64(&value_, static_cast<LONG64>(value)); }
};
#else
#include <stdint.h>
#include <atomic>
using VoiceU32 = uint32_t;
using VoiceU64 = uint64_t;
class VoiceAtomic64 {
    std::atomic<VoiceU64> value_{0};
public:
    operator VoiceU64() const { return Load(); }
    VoiceAtomic64& operator=(VoiceU64 value) { Store(value); return *this; }
    bool TryEnter() { VoiceU64 expected = 0; return value_.compare_exchange_strong(expected, 1, std::memory_order_acq_rel); }
    VoiceU64 Load() const { return value_.load(std::memory_order_acquire); }
    void Store(VoiceU64 value) { value_.store(value, std::memory_order_release); }
};
#endif

static_assert(sizeof(VoiceU32) == 4 && sizeof(VoiceU64) == 8, "Voice PCM and clock integer widths must be fixed");
inline constexpr VoiceU64 VoiceMaxU64 = ~static_cast<VoiceU64>(0);

// One render DPC produces; one capture DPC consumes. Control never resets either
// owner's counter. Ring samples are safe to reuse only after the consumer release.
class VoiceBridge {
public:
    static constexpr VoiceU32 Capacity = 4096;
private:
    VoiceU32 samples_[Capacity]{};
    VoiceAtomic64 written_, read_, generation_, live_, shutdown_;
    VoiceU64 captureGeneration_ = 0; // capture DPC only
public:
    // Called with the render timer quiescent, before publishing RUN.
    void StartProducer() {
        if (shutdown_.Load()) return;
        generation_.Store(generation_.Load() + 1); live_.Store(1);
        if (shutdown_.Load()) live_.Store(0);
    }
    // May be called from control: a concurrent capture block checks live again.
    void StopProducer() { live_.Store(0); }
    void Close() { shutdown_.Store(1); live_.Store(0); }
    bool Live() const { return !shutdown_.Load() && live_.Load() != 0; }
    // Capture owner only, also legal at PASSIVE with its DPC quiescent.
    void DropConsumer() {
        read_.Store(written_.Load());
        captureGeneration_ = generation_.Load();
    }
    VoiceU32 Push(const VoiceU32* source, VoiceU32 frames) {
        if (!Live() || frames > Capacity) return 0;
        const auto write = written_.Load(), read = read_.Load();
        const auto used = write - read;
        if (used > Capacity) return 0;
        const auto room = Capacity - static_cast<VoiceU32>(used);
        const auto count = frames < room ? frames : room;
        for (VoiceU32 i = 0; i < count; ++i) samples_[(write + i) % Capacity] = source[i];
        written_.Store(write + count);
        return count; // Overflow drops NEW frames, never overwrites unread storage.
    }
    VoiceU32 Pop(VoiceU32* destination, VoiceU32 frames) {
        if (frames > Capacity) return 0;
        const auto generation = generation_.Load();
        const auto write = written_.Load();
        if (!Live() || captureGeneration_ != generation) {
            read_.Store(write);
            captureGeneration_ = generation;
            for (VoiceU32 i = 0; i < frames; ++i) destination[i] = 0;
            return 0;
        }
        const auto read = read_.Load();
        const auto available = write - read;
        const auto count = available <= Capacity ? (frames < available ? frames : static_cast<VoiceU32>(available)) : 0;
        for (VoiceU32 i = 0; i < count; ++i) destination[i] = samples_[(read + i) % Capacity];
        for (VoiceU32 i = count; i < frames; ++i) destination[i] = 0;
        read_.Store(available <= Capacity ? read + count : write);
        if (!Live() || generation_.Load() != generation) {
            for (VoiceU32 i = 0; i < frames; ++i) destination[i] = 0;
            return 0;
        }
        return count;
    }
};

// Integer QPC conversion shared by portable tests and the kernel DPC. Carries
// fractions so 1 ms timers do not accumulate truncation drift. Long scheduling
// gaps are marked discontinuities instead of causing unbounded catch-up loops.
class VoiceFrameClock {
    VoiceU64 previous_ = 0, carry_ = 0, frequency_ = 1;
public:
    void Reset(VoiceU64 now, VoiceU64 frequency) { previous_ = now; frequency_ = frequency ? frequency : 1; carry_ = 0; }
    VoiceU64 Advance(VoiceU64 now, bool& discontinuity) {
        discontinuity = now < previous_;
        const auto delta = now >= previous_ ? now - previous_ : 0;
        previous_ = now;
        if (delta > frequency_) { discontinuity = true; carry_ = 0; return 48000; }
        const auto scaled = delta * 48000 + carry_;
        const auto result = scaled / frequency_;
        carry_ = scaled % frequency_;
        return result;
    }
};
