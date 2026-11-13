// stitchup-ocr — text recognition with Apple's Vision framework (on-device).
//
//   stitchup-ocr <image> [language,language,...]
//
// Prints a JSON array of recognised lines:
//   [{"text": "...", "x": 0.1, "y": 0.8, "w": 0.5, "h": 0.03, "confidence": 0.98}, ...]
// Coordinates are normalised (0..1) with the origin at the BOTTOM-left, the
// same orientation as PDF user space.
import Foundation
import Vision
import ImageIO

func fail(_ msg: String) -> Never {
    FileHandle.standardError.write((msg + "\n").data(using: .utf8)!)
    exit(1)
}

let args = CommandLine.arguments
guard args.count >= 2 else { fail("usage: stitchup-ocr <image> [languages]") }
let url = URL(fileURLWithPath: args[1])
guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(src, 0, nil) else { fail("cannot read image \(args[1])") }

@available(macOS 10.15, *)
func recognize(_ image: CGImage) {

    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true
    if args.count >= 3 {
        request.recognitionLanguages = args[2].split(separator: ",").map(String.init)
    } else if #available(macOS 13.0, *) {
        request.automaticallyDetectsLanguage = true
    }

    do {
        try VNImageRequestHandler(cgImage: image, options: [:]).perform([request])
    } catch {
        fail("recognition failed: \(error.localizedDescription)")
    }

    var out: [[String: Any]] = []
    for obs in request.results ?? [] {
        guard let best = obs.topCandidates(1).first else { continue }
        let b = obs.boundingBox
        out.append(["text": best.string, "x": b.minX, "y": b.minY, "w": b.width, "h": b.height, "confidence": best.confidence])
    }
    let data = try! JSONSerialization.data(withJSONObject: out, options: [])
    FileHandle.standardOutput.write(data)
}

if #available(macOS 10.15, *) {
    recognize(image)
} else {
    fail("text recognition needs macOS 10.15 or later")
}
