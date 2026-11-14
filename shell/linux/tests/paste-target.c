/* A real GTK editor used by the isolated Linux dictation smoke test. It saves
   its text to the first path and, given a second, logs each Escape there.
   With --password before the paths, the editor is a password field. */
#include <gtk/gtk.h>

static gboolean password;
static const char *result_path;
static const char *escapes_path;

static void save(const char *text) {
  GError *error = NULL;
  if (!g_file_set_contents(result_path, text, -1, &error)) {
    g_printerr("Could not save target text: %s\n", error->message);
    g_error_free(error);
  }
}

static void changed(GtkTextBuffer *buffer, gpointer data) {
  GtkTextIter start, end;
  gtk_text_buffer_get_bounds(buffer, &start, &end);
  char *text = gtk_text_buffer_get_text(buffer, &start, &end, FALSE);
  save(text);
  g_free(text);
}

static void password_changed(GtkEditable *editable, gpointer data) {
  save(gtk_editable_get_text(editable));
}

static GtkWidget *editor(void) {
  if (password) {
    GtkWidget *entry = gtk_password_entry_new();
    g_signal_connect(entry, "changed", G_CALLBACK(password_changed), NULL);
    return entry;
  }
  GtkWidget *view = gtk_text_view_new();
  g_signal_connect(gtk_text_view_get_buffer(GTK_TEXT_VIEW(view)), "changed",
                   G_CALLBACK(changed), NULL);
  return view;
}

/* Records each Escape that reaches this application. */
static gboolean key_pressed(GtkEventControllerKey *controller, guint keyval,
                            guint keycode, GdkModifierType state,
                            gpointer data) {
  if (keyval != GDK_KEY_Escape) return FALSE;
  FILE *escapes = fopen(escapes_path, "a");
  if (!escapes || fputs("Escape\n", escapes) == EOF) {
    g_printerr("Could not record Escape\n");
  }
  if (escapes) fclose(escapes);
  return FALSE;
}

static void activate(GtkApplication *app, gpointer data) {
  GtkWidget *window = gtk_application_window_new(app);
  gtk_window_set_title(GTK_WINDOW(window), "Greendale dictation target");
  gtk_window_set_default_size(GTK_WINDOW(window), 500, 300);
  GtkWidget *view = editor();
  gtk_window_set_child(GTK_WINDOW(window), view);
  if (escapes_path) {
    GtkEventController *keys = gtk_event_controller_key_new();
    gtk_event_controller_set_propagation_phase(keys, GTK_PHASE_CAPTURE);
    g_signal_connect(keys, "key-pressed", G_CALLBACK(key_pressed), NULL);
    gtk_widget_add_controller(window, keys);
  }
  gtk_window_present(GTK_WINDOW(window));
  gtk_widget_grab_focus(view);
}

int main(int argc, char **argv) {
  char **paths = argv + 1;
  int count = argc - 1;
  if (count > 0 && g_str_equal(paths[0], "--password")) {
    password = TRUE;
    paths++;
    count--;
  }
  if (count != 1 && count != 2) return 2;
  result_path = paths[0];
  escapes_path = count == 2 ? paths[1] : NULL;
  GtkApplication *app = gtk_application_new("com.rselbach.onda.TestTarget",
                                           G_APPLICATION_NON_UNIQUE);
  g_signal_connect(app, "activate", G_CALLBACK(activate), NULL);
  int status = g_application_run(G_APPLICATION(app), 1, argv);
  g_object_unref(app);
  return status;
}
