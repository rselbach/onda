/* A real GTK editor used by the isolated Linux dictation smoke test. */
#include <gtk/gtk.h>

static const char *result_path;

static void changed(GtkTextBuffer *buffer, gpointer data) {
  GtkTextIter start, end;
  gtk_text_buffer_get_bounds(buffer, &start, &end);
  char *text = gtk_text_buffer_get_text(buffer, &start, &end, FALSE);
  GError *error = NULL;
  if (!g_file_set_contents(result_path, text, -1, &error)) {
    g_printerr("Could not save target text: %s\n", error->message);
    g_error_free(error);
  }
  g_free(text);
}

static void activate(GtkApplication *app, gpointer data) {
  GtkWidget *window = gtk_application_window_new(app);
  gtk_window_set_title(GTK_WINDOW(window), "Greendale dictation target");
  gtk_window_set_default_size(GTK_WINDOW(window), 500, 300);
  GtkWidget *view = gtk_text_view_new();
  gtk_window_set_child(GTK_WINDOW(window), view);
  g_signal_connect(gtk_text_view_get_buffer(GTK_TEXT_VIEW(view)), "changed",
                   G_CALLBACK(changed), NULL);
  gtk_window_present(GTK_WINDOW(window));
  gtk_widget_grab_focus(view);
}

int main(int argc, char **argv) {
  if (argc != 2) return 2;
  result_path = argv[1];
  GtkApplication *app = gtk_application_new("com.rselbach.onda.TestTarget",
                                           G_APPLICATION_NON_UNIQUE);
  g_signal_connect(app, "activate", G_CALLBACK(activate), NULL);
  int status = g_application_run(G_APPLICATION(app), 1, argv);
  g_object_unref(app);
  return status;
}
