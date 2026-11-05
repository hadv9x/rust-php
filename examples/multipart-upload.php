<?php
$upload = $_FILES['upload'] ?? null;
$profile = $_POST['profile_name'] ?? null;
if ($profile !== null) {
    echo 'profile: ', $profile, "\n";
}

if (is_array($upload)) {
    echo 'upload error: ', $upload['error'], '; name: ', $upload['name'], '; size: ', $upload['size'], "\n";
} else {
    echo '<form action="" method="post" enctype="multipart/form-data">';
    echo '<input type="hidden" name="MAX_FILE_SIZE" value="1048576">';
    echo '<input name="profile.name" placeholder="Profile name">';
    echo '<input type="file" name="upload"><button>Upload</button></form>', "\n";
}
