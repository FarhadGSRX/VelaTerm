import UIKit
import AVFoundation

/// 相机画面仅在设备上用于二维码识别，不拍照、不保存、不上传。
final class URLScanner: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    var completion: ((String?, String?) -> Void)?
    private let session = AVCaptureSession()
    private let captureQueue = DispatchQueue(label: "com.velaterm.mobile.qr-camera")
    private var preview: AVCaptureVideoPreviewLayer!
    private var finished = false
    private var active = false // 仅由 captureQueue 访问。
    private let guide = UIView()
    private let message = UILabel()

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        preview = AVCaptureVideoPreviewLayer(session: session)
        preview.videoGravity = .resizeAspectFill
        view.layer.addSublayer(preview)
        let cancel = UIButton(type: .system)
        cancel.setTitle("取消", for: .normal)
        cancel.tintColor = .white
        cancel.backgroundColor = UIColor.black.withAlphaComponent(0.6)
        cancel.layer.cornerRadius = 12
        cancel.addTarget(self, action: #selector(cancelScan), for: .touchUpInside)
        cancel.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(cancel)
        guide.layer.borderWidth = 2
        guide.layer.borderColor = UIColor.systemMint.cgColor
        guide.layer.cornerRadius = 16
        guide.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(guide)
        message.text = "将 URL 二维码放入取景框"
        message.textColor = .white
        message.textAlignment = .center
        message.numberOfLines = 0
        message.backgroundColor = UIColor.black.withAlphaComponent(0.6)
        message.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(message)
        NSLayoutConstraint.activate([
            cancel.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 16),
            cancel.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 20),
            cancel.widthAnchor.constraint(equalToConstant: 80), cancel.heightAnchor.constraint(equalToConstant: 48),
            guide.centerXAnchor.constraint(equalTo: view.centerXAnchor), guide.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            guide.widthAnchor.constraint(equalTo: view.widthAnchor, multiplier: 0.65),
            guide.heightAnchor.constraint(equalTo: guide.widthAnchor),
            message.topAnchor.constraint(equalTo: guide.bottomAnchor, constant: 24),
            message.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            message.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24)
        ])
        NotificationCenter.default.addObserver(self, selector: #selector(backgrounded), name: UIApplication.didEnterBackgroundNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(cameraFailed), name: AVCaptureSession.runtimeErrorNotification, object: session)
        configureCamera()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        preview.frame = view.bounds
        if let connection = preview.connection, connection.isVideoRotationAngleSupported(90) {
            let angle: CGFloat
            switch view.window?.windowScene?.interfaceOrientation {
            case .landscapeLeft: angle = 0
            case .landscapeRight: angle = 180
            case .portraitUpsideDown: angle = 270
            default: angle = 90
            }
            connection.videoRotationAngle = angle
        }
    }

    private func configureCamera() {
        captureQueue.async { [weak self] in
            guard let self else { return }
            self.session.beginConfiguration()
            guard let camera = AVCaptureDevice.default(for: .video),
                  let input = try? AVCaptureDeviceInput(device: camera), self.session.canAddInput(input) else {
                self.session.commitConfiguration()
                DispatchQueue.main.async { self.finish(nil, "无法使用相机，请检查设备和相机权限") }
                return
            }
            self.session.addInput(input)
            let output = AVCaptureMetadataOutput()
            guard self.session.canAddOutput(output) else {
                self.session.commitConfiguration()
                DispatchQueue.main.async { self.finish(nil, "此设备无法识别二维码") }
                return
            }
            self.session.addOutput(output)
            output.setMetadataObjectsDelegate(self, queue: .main)
            guard output.availableMetadataObjectTypes.contains(.qr) else {
                self.session.commitConfiguration()
                DispatchQueue.main.async { self.finish(nil, "此设备不支持二维码识别") }
                return
            }
            output.metadataObjectTypes = [.qr]
            self.session.commitConfiguration()
            self.active = true
            self.session.startRunning()
        }
    }

    func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
        guard !finished else { return }
        for object in objects {
            if let code = object as? AVMetadataMachineReadableCodeObject, code.type == .qr, let text = code.stringValue {
                finish(text, nil)
                return
            }
        }
    }

    @objc private func cancelScan() { finish(nil, nil) }
    @objc private func backgrounded() { finish(nil, nil) }
    @objc private func cameraFailed() {
        DispatchQueue.main.async { [weak self] in self?.finish(nil, "相机不可用，请关闭其他使用相机的应用后重试") }
    }
    private func finish(_ value: String?, _ error: String?) {
        guard !finished else { return }
        finished = true
        captureQueue.async {
            if self.active { self.session.stopRunning(); self.active = false }
            DispatchQueue.main.async {
                self.dismiss(animated: true) { self.completion?(value, error); self.completion = nil }
            }
        }
    }
    deinit { NotificationCenter.default.removeObserver(self) }
}
