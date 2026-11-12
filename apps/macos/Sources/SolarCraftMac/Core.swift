// SPDX-License-Identifier: GPL-3.0-or-later
import CSolarCraft
import Foundation

/// Swift façade over the C ABI. Runs off the main thread; progress is delivered on the main actor.
enum Core {
    static var version: String { String(cString: sc_version()) }

    struct Result { let summary: ScAnnualSummary; let hourly: [Double] }
    enum Failure: Error { case cancelled, invalidArgument, internalError(Int32) }

    final class Job: @unchecked Sendable {
        let onProgress: @Sendable (Double) -> Void
        private let lock = NSLock()
        private var _cancelled = false
        init(onProgress: @escaping @Sendable (Double) -> Void) { self.onProgress = onProgress }
        func cancel() { lock.lock(); _cancelled = true; lock.unlock() }
        var cancelled: Bool { lock.lock(); defer { lock.unlock() }; return _cancelled }
    }

    static func quickAnnual(_ input: ScQuickInput, job: Job) throws -> Result {
        var input = input
        var out = ScAnnualSummary()
        var hourly = [Double](repeating: 0, count: 8784)
        let user = Unmanaged.passUnretained(job).toOpaque()
        let rc = hourly.withUnsafeMutableBufferPointer { buf in
            sc_quick_annual(&input, &out, buf.baseAddress, buf.count, { fraction, user in
                let job = Unmanaged<Job>.fromOpaque(user!).takeUnretainedValue()
                job.onProgress(fraction)
                return job.cancelled ? 0 : 1
            }, user)
        }
        switch rc {
        case Int32(SC_OK): return Result(summary: out, hourly: Array(hourly.prefix(Int(out.steps))))
        case Int32(SC_CANCELLED): throw Failure.cancelled
        case Int32(SC_INVALID_ARGUMENT): throw Failure.invalidArgument
        default: throw Failure.internalError(rc)
        }
    }
}
