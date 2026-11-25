// 宣传片编码器（macOS 自带 AVFoundation，不需要 ffmpeg）。
//   encode <输出.mov> <宽> <高> <fps>   从 stdin 读帧：4 字节大端长度 + JPEG/PNG 数据，编成 H.264
//   mux <视频.mov> <音频.m4a> <输出.mp4>  把配乐并进去（视频、音频都直通，不重编码）
import AVFoundation
import CoreGraphics
import Foundation
import ImageIO

func fail(_ message: String) -> Never {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
    exit(1)
}

func readExactly(_ count: Int) -> Data? {
    var data = Data()
    while data.count < count {
        let chunk = FileHandle.standardInput.readData(ofLength: count - data.count)
        if chunk.isEmpty { return data.isEmpty ? nil : data }
        data.append(chunk)
    }
    return data
}

func encode(output: String, width: Int, height: Int, fps: Int32) {
    let url = URL(fileURLWithPath: output)
    try? FileManager.default.removeItem(at: url)
    guard let writer = try? AVAssetWriter(outputURL: url, fileType: .mov) else { fail("无法创建输出文件") }
    let settings: [String: Any] = [
        AVVideoCodecKey: AVVideoCodecType.h264,
        AVVideoWidthKey: width,
        AVVideoHeightKey: height,
        AVVideoCompressionPropertiesKey: [
            AVVideoAverageBitRateKey: 10_000_000,
            AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
            AVVideoMaxKeyFrameIntervalKey: Int(fps) * 2,
            AVVideoAllowFrameReorderingKey: true,
        ],
        AVVideoColorPropertiesKey: [
            AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2,
            AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
            AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2,
        ],
    ]
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: settings)
    input.expectsMediaDataInRealTime = false
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: width,
        kCVPixelBufferHeightKey as String: height,
    ])
    writer.add(input)
    writer.startWriting()
    writer.startSession(atSourceTime: .zero)
    let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!
    var frame: Int64 = 0
    while let header = readExactly(4), header.count == 4 {
        let length = header.reduce(0) { ($0 << 8) | Int($1) }
        guard let bytes = readExactly(length), bytes.count == length else { fail("帧数据不完整") }
        guard let source = CGImageSourceCreateWithData(bytes as CFData, nil),
              let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { fail("第 \(frame) 帧解码失败") }
        while !input.isReadyForMoreMediaData { usleep(2000) }
        guard let pool = adaptor.pixelBufferPool else { fail("没有像素缓冲池：\(String(describing: writer.error))") }
        var buffer: CVPixelBuffer?
        CVPixelBufferPoolCreatePixelBuffer(nil, pool, &buffer)
        guard let pixels = buffer else { fail("无法分配像素缓冲") }
        CVPixelBufferLockBaseAddress(pixels, [])
        let context = CGContext(
            data: CVPixelBufferGetBaseAddress(pixels), width: width, height: height, bitsPerComponent: 8,
            bytesPerRow: CVPixelBufferGetBytesPerRow(pixels), space: colorSpace,
            bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)!
        context.interpolationQuality = .high
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        CVPixelBufferUnlockBaseAddress(pixels, [])
        if !adaptor.append(pixels, withPresentationTime: CMTime(value: frame, timescale: fps)) {
            fail("写入第 \(frame) 帧失败：\(String(describing: writer.error))")
        }
        frame += 1
    }
    input.markAsFinished()
    let done = DispatchSemaphore(value: 0)
    writer.finishWriting { done.signal() }
    done.wait()
    if writer.status != .completed { fail("编码失败：\(String(describing: writer.error))") }
    print("已编码 \(frame) 帧 → \(output)")
}

func mux(video: String, audio: String, output: String) {
    let composition = AVMutableComposition()
    let videoAsset = AVURLAsset(url: URL(fileURLWithPath: video))
    let audioAsset = AVURLAsset(url: URL(fileURLWithPath: audio))
    let videoTrack = videoAsset.tracks(withMediaType: .video).first!
    let audioTrack = audioAsset.tracks(withMediaType: .audio).first!
    let range = CMTimeRange(start: .zero, duration: videoAsset.duration)
    let audioRange = CMTimeRange(start: .zero, duration: CMTimeMinimum(videoAsset.duration, audioAsset.duration))
    let v = composition.addMutableTrack(withMediaType: .video, preferredTrackID: kCMPersistentTrackID_Invalid)!
    let a = composition.addMutableTrack(withMediaType: .audio, preferredTrackID: kCMPersistentTrackID_Invalid)!
    try! v.insertTimeRange(range, of: videoTrack, at: .zero)
    try! a.insertTimeRange(audioRange, of: audioTrack, at: .zero)
    let url = URL(fileURLWithPath: output)
    try? FileManager.default.removeItem(at: url)
    guard let export = AVAssetExportSession(asset: composition, presetName: AVAssetExportPresetPassthrough) else { fail("无法创建导出会话") }
    export.outputURL = url
    export.outputFileType = .mp4
    export.shouldOptimizeForNetworkUse = true
    let done = DispatchSemaphore(value: 0)
    export.exportAsynchronously { done.signal() }
    done.wait()
    if export.status != .completed { fail("合成失败：\(String(describing: export.error))") }
    print("已合成 → \(output)")
}

let args = CommandLine.arguments
switch args.count > 1 ? args[1] : "" {
case "encode" where args.count == 6:
    encode(output: args[2], width: Int(args[3])!, height: Int(args[4])!, fps: Int32(args[5])!)
case "mux" where args.count == 5:
    mux(video: args[2], audio: args[3], output: args[4])
default:
    fail("用法：encoder encode <out.mov> <w> <h> <fps> | encoder mux <video.mov> <audio.m4a> <out.mp4>")
}
