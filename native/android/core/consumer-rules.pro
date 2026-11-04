# UniFFI bindings call into libphotovault_core.so through JNA, which looks classes and fields up by name.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
-keep class app.photovault.core.** { *; }
-dontwarn java.awt.**
-dontwarn com.sun.jna.**
