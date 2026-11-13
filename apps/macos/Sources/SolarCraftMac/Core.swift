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

// MARK: - JSON application API (sc_invoke)

struct CoreError: Error, LocalizedError {
    let code: String
    let message: String
    var errorDescription: String? { message }
}

extension Core {
    final class Progress: @unchecked Sendable {
        let handler: (@Sendable (Double) -> Void)?
        private let lock = NSLock()
        private var cancelled = false
        init(_ handler: (@Sendable (Double) -> Void)? = nil) { self.handler = handler }
        func cancel() { lock.lock(); cancelled = true; lock.unlock() }
        var isCancelled: Bool { lock.lock(); defer { lock.unlock() }; return cancelled }
    }

    /// Call an application method; returns the raw JSON of `result`.
    static func invokeRaw(_ method: String, _ params: [String: Any], progress: Progress? = nil) throws -> Data {
        let request = try JSONSerialization.data(withJSONObject: ["method": method, "params": params])
        let requestString = String(decoding: request, as: UTF8.self)
        let box = progress ?? Progress()
        let user = Unmanaged.passUnretained(box).toOpaque()
        guard let raw = requestString.withCString({ sc_invoke($0, { fraction, user in
            let p = Unmanaged<Core.Progress>.fromOpaque(user!).takeUnretainedValue()
            p.handler?(fraction)
            return p.isCancelled ? 0 : 1
        }, user) }) else { throw CoreError(code: "INTERNAL_ERROR", message: "null response") }
        defer { sc_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        guard let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw CoreError(code: "INTERNAL_ERROR", message: "bad response") }
        if (obj["ok"] as? Bool) == true {
            return try JSONSerialization.data(withJSONObject: obj["result"] ?? NSNull(), options: [.fragmentsAllowed])
        }
        let err = obj["error"] as? [String: Any]
        throw CoreError(code: err?["code"] as? String ?? "ERROR", message: err?["message"] as? String ?? "Unknown error")
    }

    static func invoke<T: Decodable>(_ method: String, _ params: [String: Any], as type: T.Type, progress: Progress? = nil) throws -> T {
        try JSONDecoder().decode(T.self, from: invokeRaw(method, params, progress: progress))
    }
}
