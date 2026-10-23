// SPDX-License-Identifier: MIT OR Apache-2.0
#include "../src/voicebridge.h"
#include <cassert>
#include <cstdlib>
#include <new>
#include <thread>

static std::atomic<unsigned> allocations{0};
void* operator new(std::size_t size) {
    allocations.fetch_add(1);
    if (void* memory = std::malloc(size)) return memory;
    throw std::bad_alloc();
}
void operator delete(void* memory) noexcept { std::free(memory); }
void operator delete(void* memory, std::size_t) noexcept { std::free(memory); }

int main() {
    static_assert(VoiceBridge::Capacity == 4096);
    static_assert(sizeof(VoiceBridge) < 17 * 1024);
    VoiceBridge bridge;
    uint32_t input[VoiceBridge::Capacity]{}, output[VoiceBridge::Capacity]{};
    for (uint32_t i = 0; i < VoiceBridge::Capacity; ++i) input[i] = i + 1;
    // No producer: capture is always silence, even if destination held old speech.
    output[0] = 123;
    assert(bridge.Pop(output, 1) == 0 && output[0] == 0);
    assert(bridge.Push(input, 1) == 0);
    bridge.StartProducer();
    assert(bridge.Pop(output, 1) == 0); // establish capture generation
    assert(bridge.Push(input, VoiceBridge::Capacity) == VoiceBridge::Capacity);
    assert(bridge.Push(input, 1) == 0); // full: never overwrite unread samples
    assert(bridge.Pop(output, 37) == 37);
    for (uint32_t i = 0; i < 37; ++i) assert(output[i] == i + 1);
    assert(bridge.Push(input, 37) == 37); // wraps the write pointer
    assert(bridge.Pop(output, VoiceBridge::Capacity) == VoiceBridge::Capacity);
    for (uint32_t i = 0; i < VoiceBridge::Capacity - 37; ++i) assert(output[i] == i + 38);
    for (uint32_t i = VoiceBridge::Capacity - 37; i < VoiceBridge::Capacity; ++i)
        assert(output[i] == i - (VoiceBridge::Capacity - 37) + 1);
    assert(bridge.Pop(output, 12) == 0);
    for (uint32_t i = 0; i < 12; ++i) assert(output[i] == 0);
    assert(bridge.Push(input, 4) == 4);
    bridge.StopProducer();
    assert(bridge.Pop(output, 4) == 0);
    for (uint32_t i = 0; i < 4; ++i) assert(output[i] == 0);
    bridge.StartProducer();
    assert(bridge.Pop(output, 4) == 0); // restart cannot replay queued speech
    input[0] = 0x7fff8000;
    assert(bridge.Push(input, 1) == 1);
    assert(bridge.Pop(output, 4) == 1 && output[0] == input[0]);
    for (uint32_t i = 1; i < 4; ++i) assert(output[i] == 0);
    const auto allocationsBefore = allocations.load();
    for (unsigned i = 0; i < 100000; ++i) {
        assert(bridge.Push(input, 1) == 1);
        assert(bridge.Pop(output, 1) == 1);
    }
    assert(allocations.load() == allocationsBefore);
    bridge.Close();
    bridge.StartProducer();
    assert(!bridge.Live() && bridge.Push(input, 1) == 0);
    assert(bridge.Pop(output, 1) == 0 && output[0] == 0);

    VoiceFrameClock clock;
    assert(clock.Reset(0, 10000000));
    bool discontinuity = false;
    uint64_t frames = 0;
    for (uint64_t time = 997; time <= 10000000; time += 997)
        frames += clock.Advance(time, discontinuity);
    frames += clock.Advance(10000000, discontinuity);
    assert(!discontinuity && frames == 48000);
    assert(clock.Advance(50000000, discontinuity) == 48000 && discontinuity);
    assert(clock.Advance(1, discontinuity) == 0 && discontinuity);
    assert(clock.Reset(0, 7));
    frames = 0;
    for (uint64_t i = 1; i <= 7; ++i) frames += clock.Advance(i, discontinuity);
    assert(frames == 48000);

    // Valid counter rates above 1 GHz must retain fractional frame accuracy.
    constexpr uint64_t fastFrequency = 3200000000ULL;
    assert(clock.Reset(0, fastFrequency));
    frames = 0;
    for (uint64_t time = 997001; time < fastFrequency; time += 997001) {
        frames += clock.Advance(time, discontinuity);
        assert(!discontinuity);
    }
    frames += clock.Advance(fastFrequency, discontinuity);
    assert(!discontinuity && frames == 48000);

    // Prime carry to within 48000 ticks of the largest valid denominator, then
    // exercise the full delta plus that carry at the proven arithmetic limit.
    constexpr auto maximumFrequency = VoiceFrameClock::MaxFrequency;
    constexpr auto prime = maximumFrequency / 48000;
    assert(clock.Reset(0, maximumFrequency));
    assert(clock.Advance(prime, discontinuity) == 0 && !discontinuity);
    assert(clock.Advance(prime + maximumFrequency, discontinuity) == 48000 && !discontinuity);
    assert(clock.Advance(prime + maximumFrequency * 2, discontinuity) == 48000 && !discontinuity);
    assert(!clock.Reset(0, maximumFrequency + 1));
    assert(clock.Advance(VoiceMaxU64, discontinuity) == 0 && discontinuity);
    assert(!clock.Reset(0, 0));
    assert(clock.Advance(1, discontinuity) == 0 && discontinuity);
    assert(clock.Reset(0, fastFrequency));
    assert(clock.Advance(fastFrequency, discontinuity) == 48000 && !discontinuity);

    // Exercise the actual SPSC algorithm across cores, not a model of it.
    VoiceBridge concurrent;
    concurrent.StartProducer();
    assert(concurrent.Pop(output, 1) == 0);
    constexpr uint32_t total = 100000;
    std::thread producer([&] {
        for (uint32_t i = 1; i <= total; ++i)
            while (concurrent.Push(&i, 1) == 0) std::this_thread::yield();
    });
    for (uint32_t expected = 1; expected <= total;) {
        uint32_t sample = 0;
        if (concurrent.Pop(&sample, 1) == 1) { assert(sample == expected); ++expected; }
        else std::this_thread::yield();
    }
    producer.join();
    concurrent.StopProducer();
    return 0;
}
