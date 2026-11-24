package com.auroragba

import android.app.Activity
import android.app.AlertDialog
import android.app.Dialog
import android.graphics.BitmapFactory
import android.graphics.drawable.BitmapDrawable
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.KeyEvent
import android.widget.FrameLayout
import android.widget.GridLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView

/**
 * Salvar/Carregar estado em tela cheia: abas Salvar | Carregar e a grade dos 8
 * slots com miniatura e data. Salvar num slot ocupado pede confirmação; na aba
 * Carregar os slots vazios ficam apagados. O jogo fica pausado enquanto ela
 * está aberta (o host cuida disso).
 */
class StatesScreen(private val act: Activity, private val host: Host, startOnLoad: Boolean) {
    private val ui = Ui(act)
    private var loadTab = startOnLoad
    private var closingToMenu = false

    private val body = ui.box(true, 8).apply {
        setPadding(ui.dp(16), ui.dp(8), ui.dp(16), ui.dp(24))
    }

    private val dialog = Dialog(act, android.R.style.Theme_Black_NoTitleBar_Fullscreen).apply {
        setOnKeyListener { _, keyCode, event ->
            if (keyCode == KeyEvent.KEYCODE_BACK && event.action == KeyEvent.ACTION_UP) {
                closeToMenu()
            }
            keyCode == KeyEvent.KEYCODE_BACK
        }
        setOnDismissListener { host.onStatesClosed(closingToMenu) }
    }

    private fun t(id: Int, vararg args: Any) = host.tr(id, *args)

    fun show() {
        val backButton = FrameLayout(act).apply {
            background = ui.touchable(ui.rounded(0x00000000, 22))
            contentDescription = t(R.string.back)
            addView(ui.icon(R.drawable.ic_back, Pal.TEXT, 22).apply {
                layoutParams = FrameLayout.LayoutParams(ui.dp(22), ui.dp(22), Gravity.CENTER)
            })
            setOnClickListener { closeToMenu() }
        }
        val header = ui.box(false, 8).apply {
            gravity = Gravity.CENTER_VERTICAL
            setPadding(ui.dp(12), ui.dp(16), ui.dp(12), ui.dp(8))
            addView(backButton, LinearLayout.LayoutParams(ui.dp(44), ui.dp(44)))
            addView(ui.text(t(R.string.states_title), 22f, bold = true))
        }
        val root = ui.box(true).apply {
            setBackgroundColor(Pal.BG)
            addView(header)
            addView(ScrollView(act).apply { addView(body) }, ui.lp(Ui.MATCH, 0, 1f))
        }
        dialog.setContentView(root)
        render()
        dialog.show()
    }

    private fun closeToMenu() {
        closingToMenu = true
        dialog.dismiss()
    }

    private fun render() {
        body.removeAllViews()
        body.addView(ui.segmented(listOf(t(R.string.states_save), t(R.string.states_load)), if (loadTab) 1 else 0) {
            loadTab = it == 1
            render()
        })
        body.addView(ui.text(t(if (loadTab) R.string.states_hint_load else R.string.states_hint_save), 13f, Pal.MUTED).apply {
            setPadding(ui.dp(4), ui.dp(6), ui.dp(4), ui.dp(4))
        })
        body.addView(grid())
    }

    private fun grid() = GridLayout(act).apply {
        columnCount = 2
        val gap = ui.dp(6)
        host.stateSlots().forEachIndexed { i, slot ->
            addView(tile(i, slot), GridLayout.LayoutParams(
                GridLayout.spec(GridLayout.UNDEFINED), GridLayout.spec(GridLayout.UNDEFINED, 1f),
            ).apply {
                width = 0
                setMargins(gap, gap, gap, gap)
            })
        }
    }

    private fun tile(i: Int, slot: SlotInfo) = ui.box(true, 8).apply {
        val disabled = loadTab && !slot.exists
        background = ui.touchable(ui.rounded(Pal.SURFACE, 14))
        setPadding(ui.dp(8), ui.dp(8), ui.dp(8), ui.dp(8))
        alpha = if (disabled) 0.4f else 1f
        isEnabled = !disabled
        contentDescription = t(R.string.states_slot, i + 1) +
            if (slot.exists) ", " + host.slotDate(slot.modified) else ", " + t(R.string.states_empty)

        addView(AspectFrame(act, 2f / 3f).apply { addView(preview(slot), FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH)) })
        addView(ui.box(false, 6).apply {
            gravity = Gravity.CENTER_VERTICAL
            setPadding(ui.dp(2), 0, ui.dp(2), 0)
            addView(ui.text(t(R.string.states_slot, i + 1), 14f, bold = true), ui.lp(0, Ui.WRAP, 1f))
            if (slot.exists) addView(ui.text(host.slotDate(slot.modified), 12f, Pal.MUTED))
        })
        if (!disabled) setOnClickListener { pick(i, slot) }
    }

    /** Miniatura (pixels nítidos), "sem miniatura" ou o quadro tracejado de vazio. */
    private fun preview(slot: SlotInfo): android.view.View {
        val bmp = slot.thumb?.takeIf { slot.exists && it.isFile }?.let {
            try { BitmapFactory.decodeFile(it.absolutePath) } catch (e: Exception) { null }
        }
        if (bmp != null) {
            return ImageView(act).apply {
                scaleType = ImageView.ScaleType.FIT_XY
                setImageDrawable(BitmapDrawable(act.resources, bmp).apply { isFilterBitmap = false })
                background = ui.rounded(0xFF1E2A36.toInt(), 8)
                clipToOutline = true
            }
        }
        val label = ui.text(t(if (slot.exists) R.string.states_no_preview else R.string.states_empty), 12f, Pal.FAINT)
        label.gravity = Gravity.CENTER
        label.background = if (slot.exists) {
            ui.rounded(0xFF1E2A36.toInt(), 8)
        } else {
            GradientDrawable().apply {
                cornerRadius = ui.dp(8).toFloat()
                setStroke(ui.dp(1), 0xFF3A3A45.toInt(), ui.dp(5).toFloat(), ui.dp(4).toFloat())
            }
        }
        return label
    }

    private fun pick(i: Int, slot: SlotInfo) {
        if (loadTab) {
            host.loadSlot(i)
            dialog.dismiss()
        } else if (slot.exists) {
            AlertDialog.Builder(act)
                .setTitle(t(R.string.states_overwrite_title, i + 1))
                .setMessage(t(R.string.states_overwrite_msg, host.slotDate(slot.modified)))
                .setPositiveButton(t(R.string.states_overwrite)) { _, _ ->
                    host.saveSlot(i)
                    dialog.dismiss()
                }
                .setNegativeButton(t(R.string.cancel), null)
                .show()
        } else {
            host.saveSlot(i)
            dialog.dismiss()
        }
    }
}
