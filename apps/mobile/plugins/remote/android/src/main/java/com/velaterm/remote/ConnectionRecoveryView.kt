package com.velaterm.remote

import android.content.Context
import android.view.Gravity
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

/** Navigation independent of the failed remote document. */
class ConnectionRecoveryView(context: Context) : ScrollView(context) {
    private val strings = MobileText(context)
    val retryButton = Button(context).apply { text = strings.get("common.retry") }
    val backButton = Button(context).apply { text = strings.get("mobile.backConnections") }
    private val detail = TextView(context).apply { textSize = 17f; gravity = Gravity.CENTER }
    private val title = TextView(context).apply { textSize = 22f; gravity = Gravity.CENTER }
    private val progress = android.widget.ProgressBar(context)
    var onRetry: (() -> Unit)? = null
    var onBack: (() -> Unit)? = null
    init {
        val density = resources.displayMetrics.density
        fun dp(value: Int) = (density * value).toInt()
        val background = android.util.TypedValue(); context.theme.resolveAttribute(android.R.attr.colorBackground, background, true)
        setBackgroundColor(background.data); visibility = GONE; isFillViewport = true
        val content = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER_VERTICAL; setPadding(dp(24), dp(24), dp(24), dp(24)) }
        addView(content, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
        content.addView(progress, LinearLayout.LayoutParams(dp(28), dp(28)).apply { gravity = Gravity.CENTER_HORIZONTAL; bottomMargin = dp(16) })
        listOf(title, detail, retryButton, backButton).forEachIndexed { index, child ->
            content.addView(child, LinearLayout.LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT).apply { if (index > 0) topMargin = dp(16) })
        }
        retryButton.minHeight = dp(48); backButton.minHeight = dp(48)
        retryButton.setOnClickListener { onRetry?.invoke() }; backButton.setOnClickListener { onBack?.invoke() }
    }
    fun showLoading(message: String = "", allowRetry: Boolean = false) {
        title.text = strings.get("common.loading")
        detail.text = message; detail.visibility = if (message.isEmpty()) GONE else VISIBLE
        progress.visibility = VISIBLE
        retryButton.visibility = if (allowRetry) VISIBLE else GONE; retryButton.isEnabled = allowRetry
        backButton.isEnabled = true; visibility = VISIBLE
    }
    fun show(message: String, busy: Boolean = false) {
        if (busy) { showLoading(message); return }
        title.text = strings.get("mobile.connectionUnavailable")
        detail.text = message; detail.visibility = if (message.isEmpty()) GONE else VISIBLE
        progress.visibility = GONE
        retryButton.visibility = VISIBLE; retryButton.isEnabled = true
        backButton.isEnabled = true; visibility = VISIBLE
    }
}
