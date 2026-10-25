package com.ifeanyiibekie.tungstate

import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Edge to edge stops Android shrinking the window for the keyboard, so
    // the page is padded by the keyboard's height instead. Without this the
    // keyboard covers the buttons at the bottom of a form.
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      view.setPadding(0, 0, 0, insets.getInsets(WindowInsetsCompat.Type.ime()).bottom)
      insets
    }
  }
}
