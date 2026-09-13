import UIKit

/// Native recovery remains usable when no remote document can provide navigation.
final class ConnectionRecoveryView: UIView {
    let retryButton = UIButton(type: .system)
    let backButton = UIButton(type: .system)
    private let detail = UILabel()
    private let title = UILabel()
    private let progress = UIActivityIndicatorView(style: .medium)
    var onRetry: (() -> Void)?
    var onBack: (() -> Void)?
    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .systemBackground
        isHidden = true
        let scroll = UIScrollView(); scroll.translatesAutoresizingMaskIntoConstraints = false; addSubview(scroll)
        let content = UIView(); content.translatesAutoresizingMaskIntoConstraints = false; scroll.addSubview(content)
        let stack = UIStackView(); stack.axis = .vertical; stack.spacing = 16; stack.translatesAutoresizingMaskIntoConstraints = false; content.addSubview(stack)
        title.text = MobileText.get("mobile.connectionUnavailable"); title.textAlignment = .center; title.font = .preferredFont(forTextStyle: .title2); title.numberOfLines = 0
        detail.textAlignment = .center; detail.font = .preferredFont(forTextStyle: .body); detail.textColor = .secondaryLabel; detail.numberOfLines = 0
        retryButton.configuration = .filled(); retryButton.setTitle(MobileText.get("common.retry"), for: .normal)
        backButton.configuration = .tinted(); backButton.setTitle(MobileText.get("mobile.backConnections"), for: .normal)
        retryButton.accessibilityIdentifier = "connection-retry"; backButton.accessibilityIdentifier = "connection-back"
        retryButton.addAction(UIAction { [weak self] _ in self?.onRetry?() }, for: .touchUpInside)
        backButton.addAction(UIAction { [weak self] _ in self?.onBack?() }, for: .touchUpInside)
        [progress, title, detail, retryButton, backButton].forEach { stack.addArrangedSubview($0) }
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: topAnchor), scroll.bottomAnchor.constraint(equalTo: bottomAnchor), scroll.leadingAnchor.constraint(equalTo: leadingAnchor), scroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            content.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor), content.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor), content.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor), content.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor),
            content.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor), content.heightAnchor.constraint(greaterThanOrEqualTo: scroll.frameLayoutGuide.heightAnchor),
            stack.centerYAnchor.constraint(equalTo: content.centerYAnchor), stack.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 24), stack.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -24),
            stack.topAnchor.constraint(greaterThanOrEqualTo: content.topAnchor, constant: 24), stack.bottomAnchor.constraint(lessThanOrEqualTo: content.bottomAnchor, constant: -24),
            retryButton.heightAnchor.constraint(greaterThanOrEqualToConstant: 48), backButton.heightAnchor.constraint(greaterThanOrEqualToConstant: 48)
        ])
    }
    required init?(coder: NSCoder) { fatalError("Use init(frame:)") }
    func showLoading(_ text: String = "", allowRetry: Bool = false) {
        title.text = MobileText.get("common.loading")
        detail.text = text; detail.isHidden = text.isEmpty
        progress.startAnimating(); progress.isHidden = false
        retryButton.isHidden = !allowRetry; retryButton.isEnabled = allowRetry
        backButton.isEnabled = true; isHidden = false
        setNeedsLayout(); layoutIfNeeded()
    }
    func show(_ text: String, busy: Bool = false) {
        if busy { showLoading(text); return }
        title.text = MobileText.get("mobile.connectionUnavailable")
        detail.text = text; detail.isHidden = text.isEmpty
        progress.stopAnimating(); progress.isHidden = true
        retryButton.isHidden = false; retryButton.isEnabled = true
        backButton.isEnabled = true; isHidden = false
        setNeedsLayout(); layoutIfNeeded()
    }
}
