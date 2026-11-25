package com.auroragba

import android.app.Activity
import android.graphics.Bitmap
import android.graphics.drawable.BitmapDrawable
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView

/**
 * Vistas da caça em andamento (design do canvas): o painel do retrato (abaixo da
 * tela do jogo), a sobreposição da paisagem (barra de status no topo, sprites à
 * esquerda, "Parar caça" à direita) e o aviso de shiny encontrado no topo — que
 * não pausa nada: o jogo segue e o próprio shiny na tela é o indicador.
 */
class HuntViews(private val act: Activity, private val host: Host) {
    private val ui = Ui(act)
    private val handler = Handler(Looper.getMainLooper())

    /** Ao tocar "Parar caça" (retrato ou paisagem). */
    var onStop: () -> Unit = {}
    /** Ao tocar "Salvar estado" no aviso de shiny encontrado. */
    var onSaveState: () -> Unit = {}

    // Textos fixos, re-aplicados na troca de idioma (TextView → id do texto).
    private val statics = mutableListOf<Pair<TextView, Int>>()

    private fun label(res: Int, sizeSp: Float, color: Int = Pal.TEXT, bold: Boolean = false) =
        ui.text(host.tr(res), sizeSp, color, bold).also { statics += it to res }

    fun relabel() = statics.forEach { (view, res) -> view.text = host.tr(res) }

    // ── Retrato ──────────────────────────────────────────────────────────────

    private val pTarget = ui.text("", 22f, bold = true)
    private val pSpriteNormal = sprite(72)
    private val pSpriteShiny = sprite(72)
    private val pAttempts = ui.text("0", 24f, bold = true)
    private val pBest = ui.text("—", 24f, bold = true)
    private val pChance = ui.text("", 15f, Pal.ACCENT, bold = true)
    private val pBarFill = View(act).apply { background = ui.rounded(Pal.ACCENT, 4) }
    private val pBarRest = View(act)

    val panel: LinearLayout = ui.box(true, 14).apply {
        setBackgroundColor(Pal.BG)
        setPadding(ui.dp(16), ui.dp(20), ui.dp(16), ui.dp(24))

        addView(ui.box(false, 12).apply {
            gravity = Gravity.CENTER_VERTICAL
            addView(ui.box(true, 2).apply {
                addView(label(R.string.hunt_label_hunting, 12f, Pal.ACCENT, bold = true).apply { letterSpacing = 0.08f; isAllCaps = true })
                addView(pTarget)
            }, ui.lp(0, Ui.WRAP, 1f))
            addView(statusChip())
        })

        addView(ui.box(false, 10).apply {
            addView(spriteCard(pSpriteNormal, R.string.hunt_normal, highlight = false), ui.lp(0, Ui.WRAP, 1f))
            addView(spriteCard(pSpriteShiny, R.string.hunt_sprite_goal, highlight = true), ui.lp(0, Ui.WRAP, 1f))
        })

        addView(ui.box(false, 10).apply {
            addView(statCard(R.string.hunt_attempts, null, pAttempts), ui.lp(0, Ui.WRAP, 1f))
            addView(statCard(R.string.hunt_best_sv, R.string.hunt_shiny_lt8, pBest), ui.lp(0, Ui.WRAP, 1f))
        })

        addView(ui.card(8, 14).apply {
            addView(ui.box(false, 8).apply {
                gravity = Gravity.CENTER_VERTICAL
                addView(label(R.string.hunt_chance, 12f, Pal.MUTED), ui.lp(0, Ui.WRAP, 1f))
                addView(pChance)
            })
            addView(LinearLayout(act).apply {
                background = ui.rounded(Pal.DIVIDER, 4)
                addView(pBarFill, ui.lp(0, ui.dp(8), 0f))
                addView(pBarRest, ui.lp(0, ui.dp(8), 1f))
            }, ui.lp(Ui.MATCH, ui.dp(8)))
        })

        addView(View(act), ui.lp(Ui.MATCH, 0, 1f))
        addView(stopButton(), ui.lp(Ui.MATCH, ui.dp(52)))
    }

    // ── Paisagem ─────────────────────────────────────────────────────────────

    private val lTarget = ui.text("", 13f, bold = true)
    private val lAttempts = ui.text("", 13f, 0xFFD6D6DD.toInt())
    private val lBest = ui.text("", 13f, 0xFFD6D6DD.toInt())
    private val lChance = ui.text("", 13f, Pal.ACCENT, bold = true)
    private val lSpriteNormal = sprite(56)
    private val lSpriteShiny = sprite(56)

    val overlay: FrameLayout = FrameLayout(act).apply {
        val pill = ui.box(false, 16).apply {
            gravity = Gravity.CENTER_VERTICAL
            background = ui.rounded(0xD9121216.toInt(), 999, 0xFF33333D.toInt())
            setPadding(ui.dp(18), ui.dp(8), ui.dp(18), ui.dp(8))
            addView(ui.box(false, 8).apply {
                gravity = Gravity.CENTER_VERTICAL
                addView(View(act).apply { background = ui.rounded(Pal.ACCENT, 4) }, LinearLayout.LayoutParams(ui.dp(8), ui.dp(8)))
                addView(lTarget)
            })
            addView(lAttempts)
            addView(lBest)
            addView(lChance)
        }
        addView(pill, FrameLayout.LayoutParams(Ui.WRAP, Ui.WRAP, Gravity.TOP or Gravity.CENTER_HORIZONTAL).apply {
            topMargin = ui.dp(14)
        })
        addView(ui.box(true, 12).apply {
            addView(smallSpriteCard(lSpriteNormal, R.string.hunt_normal, highlight = false))
            addView(smallSpriteCard(lSpriteShiny, R.string.hunt_shiny_short, highlight = true))
        }, FrameLayout.LayoutParams(ui.dp(105), Ui.WRAP, Gravity.START or Gravity.CENTER_VERTICAL).apply {
            marginStart = ui.dp(12)
        })
        addView(stopButton(vertical = true), FrameLayout.LayoutParams(ui.dp(105), ui.dp(88), Gravity.END or Gravity.CENTER_VERTICAL).apply {
            marginEnd = ui.dp(12)
        })
    }

    // ── Aviso de shiny encontrado ────────────────────────────────────────────

    private val foundSub = ui.text("", 12f, Pal.MUTED)
    private val hideBanner = Runnable { banner.visibility = View.GONE }

    val banner: LinearLayout = ui.box(false, 10).apply {
        gravity = Gravity.CENTER_VERTICAL
        background = ui.rounded(0xF0121216.toInt(), 14, Pal.ACCENT, 2)
        setPadding(ui.dp(14), ui.dp(8), ui.dp(8), ui.dp(8))
        elevation = ui.dp(6).toFloat()
        visibility = View.GONE
        addView(ui.icon(R.drawable.ic_sparkle, Pal.ACCENT, 22))
        addView(ui.box(true, 1).apply {
            addView(label(R.string.hunt_found_title, 14f, bold = true))
            addView(foundSub)
        }, ui.lp(0, Ui.WRAP, 1f))
        addView(label(R.string.menu_save_state, 13f, Pal.ON_ACCENT, bold = true).apply {
            gravity = Gravity.CENTER
            minHeight = ui.dp(40)
            setPadding(ui.dp(12), 0, ui.dp(12), 0)
            background = ui.touchable(ui.rounded(Pal.ACCENT, 10))
            setOnClickListener {
                handler.removeCallbacks(hideBanner)
                hideBanner.run()
                onSaveState()
            }
        })
    }

    /** Mostra o aviso no topo por alguns segundos (o jogo segue rodando). */
    fun showFound(target: String, attempts: String) {
        foundSub.text = host.tr(R.string.hunt_found_sub, target, attempts)
        banner.visibility = View.VISIBLE
        handler.removeCallbacks(hideBanner)
        handler.postDelayed(hideBanner, FOUND_BANNER_MS)
    }

    /** Atualiza retrato e paisagem com as stats publicadas pela thread GL. */
    fun update(target: String, attempts: String, best: String, chance: Double, chanceText: String, normal: Bitmap?, shiny: Bitmap?) {
        pTarget.text = target
        pAttempts.text = attempts
        pBest.text = best
        pChance.text = chanceText
        val p = chance.coerceIn(0.0, 1.0).toFloat()
        (pBarFill.layoutParams as LinearLayout.LayoutParams).weight = p
        (pBarRest.layoutParams as LinearLayout.LayoutParams).weight = 1f - p
        pBarFill.requestLayout()

        lTarget.text = target
        lAttempts.text = host.tr(R.string.hunt_attempts_inline, attempts)
        lBest.text = host.tr(R.string.hunt_sv_inline, best)
        lChance.text = chanceText

        setSprite(pSpriteNormal, normal)
        setSprite(pSpriteShiny, shiny)
        setSprite(lSpriteNormal, normal)
        setSprite(lSpriteShiny, shiny)
    }

    // ── Peças ────────────────────────────────────────────────────────────────

    private fun sprite(sizeDp: Int) = ImageView(act).apply {
        scaleType = ImageView.ScaleType.FIT_CENTER
        layoutParams = LinearLayout.LayoutParams(ui.dp(sizeDp), ui.dp(sizeDp))
    }

    /** Sprite com pixels nítidos; sem sprite (selvagem antes do 1º encontro) limpa. */
    private fun setSprite(view: ImageView, bmp: Bitmap?) {
        val current = (view.drawable as? BitmapDrawable)?.bitmap
        if (current === bmp) return
        view.setImageDrawable(bmp?.let { BitmapDrawable(act.resources, it).apply { isFilterBitmap = false } })
    }

    private fun statusChip() = ui.box(false, 8).apply {
        gravity = Gravity.CENTER_VERTICAL
        background = ui.rounded(Pal.SURFACE, 999)
        setPadding(ui.dp(12), ui.dp(8), ui.dp(12), ui.dp(8))
        addView(View(act).apply { background = ui.rounded(Pal.ACCENT, 4) }, LinearLayout.LayoutParams(ui.dp(8), ui.dp(8)))
        addView(label(R.string.hunt_running, 13f, 0xFFD6D6DD.toInt(), bold = true))
    }

    private fun spriteCard(img: ImageView, labelRes: Int, highlight: Boolean) = ui.box(true, 8).apply {
        gravity = Gravity.CENTER_HORIZONTAL
        background = ui.rounded(Pal.SURFACE, 14, if (highlight) 0x5978C8FF else null)
        setPadding(ui.dp(12), ui.dp(12), ui.dp(12), ui.dp(12))
        addView(img)
        addView(label(labelRes, 13f, if (highlight) Pal.ACCENT else Pal.MUTED, bold = highlight).apply { gravity = Gravity.CENTER })
    }

    private fun smallSpriteCard(img: ImageView, labelRes: Int, highlight: Boolean) = ui.box(true, 6).apply {
        gravity = Gravity.CENTER_HORIZONTAL
        background = ui.rounded(0xE61C1C22.toInt(), 12, if (highlight) 0x5978C8FF else null)
        setPadding(ui.dp(8), ui.dp(8), ui.dp(8), ui.dp(8))
        addView(img)
        addView(label(labelRes, 12f, if (highlight) Pal.ACCENT else Pal.MUTED, bold = highlight).apply { gravity = Gravity.CENTER })
    }

    private fun statCard(titleRes: Int, noteRes: Int?, value: TextView) = ui.card(4, 12).apply {
        addView(ui.box(false, 4).apply {
            addView(label(titleRes, 12f, Pal.MUTED))
            if (noteRes != null) addView(label(noteRes, 12f, Pal.FAINT))
        })
        addView(value)
    }

    private fun stopButton(vertical: Boolean = false) = ui.box(vertical, if (vertical) 8 else 10).apply {
        gravity = Gravity.CENTER
        background = ui.touchable(ui.rounded(if (vertical) 0xD9121216.toInt() else 0x00000000, 14, STOP_COLOR, 2))
        addView(View(act).apply { background = ui.rounded(STOP_TEXT, 2) }, LinearLayout.LayoutParams(ui.dp(16), ui.dp(16)))
        addView(label(R.string.hunt_stop_plain, if (vertical) 14f else 16f, STOP_TEXT, bold = true).apply { gravity = Gravity.CENTER })
        setOnClickListener { onStop() }
    }

    private companion object {
        const val STOP_COLOR = 0xFFE0787A.toInt()
        const val STOP_TEXT = 0xFFF4A3A4.toInt()
        const val FOUND_BANNER_MS = 8000L
    }
}
